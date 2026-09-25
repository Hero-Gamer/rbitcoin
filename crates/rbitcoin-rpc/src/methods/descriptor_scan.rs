//! `scantxoutset` descriptor expansion. Stateless: derive scripts, do not store them.

use super::*;
use bitcoin::bip32::{ChildNumber, Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use miniscript::descriptor::checksum::desc_checksum;
use miniscript::descriptor::{
    Descriptor, DescriptorPublicKey, DescriptorSecretKey, KeyMap, Wildcard,
};
use serde_json::Value;
use std::str::FromStr;

/// Core `MAX_DESCRIPTOR_RANGE`: `end - begin + 1` above this is "Range is too large".
const MAX_DESCRIPTOR_RANGE: i64 = 1_000_000;
/// Point lookups, not a coins-DB scan. Above the largest `rpc_scantxoutset.py` combo.
pub(crate) const MAX_SCAN_SCRIPTS: usize = 10_000;

#[derive(Debug)]
pub(crate) struct ScanScript {
    pub script: Vec<u8>,
    pub desc: String,
}

pub(crate) fn expand_scan_objects(
    ctx: &RpcContext,
    objs: &[Value],
) -> Result<Vec<ScanScript>, Value> {
    let mut out = Vec::new();
    for o in objs {
        let (desc_str, range) = scan_object(o)?;
        if let Some(literal) = literal_scan(ctx, &desc_str)? {
            if out.len() >= MAX_SCAN_SCRIPTS {
                return Err(rpc_error(
                    ERR_INVALID_PARAMETER,
                    format!("scan exceeds {MAX_SCAN_SCRIPTS} derived scripts"),
                ));
            }
            out.push(literal);
            continue;
        }
        let expanded = expand_one(&desc_str, range)?;
        if out.len().saturating_add(expanded.len()) > MAX_SCAN_SCRIPTS {
            return Err(rpc_error(
                ERR_INVALID_PARAMETER,
                format!("scan exceeds {MAX_SCAN_SCRIPTS} derived scripts"),
            ));
        }
        out.extend(expanded);
    }
    Ok(out)
}

fn scan_object(o: &Value) -> Result<(String, Option<(i64, i64)>), Value> {
    match o {
        Value::String(s) => Ok((s.clone(), None)),
        Value::Object(m) => {
            let desc = m
                .get("desc")
                .and_then(Value::as_str)
                .ok_or_else(|| rpc_error(ERR_INVALID_PARAMETER, "scanobject desc required"))?
                .to_string();
            let range = match m.get("range") {
                None => None,
                Some(v) => Some(parse_range(v)?),
            };
            Ok((desc, range))
        }
        _ => Err(rpc_error(
            ERR_INVALID_PARAMETER,
            "scanobjects entries must be descriptor strings",
        )),
    }
}

/// Core range checks, before the descriptor is parsed (`rpc_scantxoutset.py`).
fn parse_range(v: &Value) -> Result<(i64, i64), Value> {
    if let Some(n) = json_i64(v) {
        if n < 0 || n > i64::from(i32::MAX) {
            return Err(rpc_error(ERR_INVALID_PARAMETER, "End of range is too high"));
        }
        check_span(0, n)?;
        return Ok((0, n));
    }
    let arr = v.as_array().ok_or_else(|| {
        rpc_error(
            ERR_INVALID_PARAMETER,
            "Range must be a number or [begin,end]",
        )
    })?;
    if arr.len() != 2 {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            "Range must be specified as end or as [begin,end]",
        ));
    }
    let begin = json_i64(&arr[0]).ok_or_else(|| {
        rpc_error(
            ERR_INVALID_PARAMETER,
            "Range should be greater or equal than 0",
        )
    })?;
    let end = json_i64(&arr[1])
        .ok_or_else(|| rpc_error(ERR_INVALID_PARAMETER, "End of range is too high"))?;
    if begin < 0 || end < 0 {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            "Range should be greater or equal than 0",
        ));
    }
    if end > i64::from(i32::MAX) {
        return Err(rpc_error(ERR_INVALID_PARAMETER, "End of range is too high"));
    }
    if begin > end {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            "Range specified as [begin,end] must not have begin after end",
        ));
    }
    check_span(begin, end)?;
    Ok((begin, end))
}

fn check_span(begin: i64, end: i64) -> Result<(), Value> {
    if end - begin + 1 > MAX_DESCRIPTOR_RANGE {
        Err(rpc_error(ERR_INVALID_PARAMETER, "Range is too large"))
    } else {
        Ok(())
    }
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
}

fn expand_one(desc_str: &str, range: Option<(i64, i64)>) -> Result<Vec<ScanScript>, Value> {
    let secp = Secp256k1::new();
    let variants = combo_variants(desc_str);
    let mut out = Vec::new();
    let mut any = false;
    for variant in &variants {
        match expand_variant(&secp, variant, range, &mut out) {
            Ok(()) => any = true,
            Err(_) if variants.len() > 1 => continue,
            Err(e) => return Err(e),
        }
        if out.len() > MAX_SCAN_SCRIPTS {
            return Err(rpc_error(
                ERR_INVALID_PARAMETER,
                format!("scan exceeds {MAX_SCAN_SCRIPTS} derived scripts"),
            ));
        }
    }
    if !any {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            format!("Invalid descriptor '{desc_str}'"),
        ));
    }
    Ok(out)
}

/// `raw(hex)` and `addr(address)` are Core descriptors miniscript does not parse.
fn literal_scan(ctx: &RpcContext, desc_str: &str) -> Result<Option<ScanScript>, Value> {
    let bare = strip_checksum(desc_str);
    let script = if bare.starts_with("raw(") {
        super::mine::parse_raw_descriptor(desc_str)
    } else if bare.starts_with("addr(") {
        super::mine::parse_addr_descriptor(ctx, desc_str)
    } else {
        return Ok(None);
    };
    let Some(script) = script else {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            format!("Invalid descriptor '{desc_str}'"),
        ));
    };
    let sum = desc_checksum(&bare)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    Ok(Some(ScanScript {
        script: script.as_bytes().to_vec(),
        desc: format!("{bare}#{sum}"),
    }))
}

/// `combo()` is not a miniscript descriptor. Core expands it to pkh, wpkh, and sh(wpkh).
fn combo_variants(desc_str: &str) -> Vec<String> {
    let bare = desc_str.split('#').next().unwrap_or(desc_str).trim();
    let Some(inner) = bare
        .strip_prefix("combo(")
        .and_then(|r| r.strip_suffix(')'))
    else {
        return vec![desc_str.to_string()];
    };
    vec![
        format!("pkh({inner})"),
        format!("wpkh({inner})"),
        format!("sh(wpkh({inner}))"),
    ]
}

fn expand_variant(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    desc_str: &str,
    range: Option<(i64, i64)>,
    out: &mut Vec<ScanScript>,
) -> Result<(), Value> {
    let (desc, keymap) = Descriptor::<DescriptorPublicKey>::parse_descriptor(secp, desc_str)
        .map_err(|e| {
            rpc_error(
                ERR_INVALID_PARAMETER,
                format!("Invalid descriptor '{desc_str}' ({e})"),
            )
        })?;
    let parts = desc
        .into_single_descriptors()
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    for part in parts {
        let ranged = part.has_wildcard();
        let source = part.to_string();
        if ranged && range.is_none() {
            push_range(secp, &part, &source, &keymap, 0, 1000, out)?;
        } else if !ranged {
            if range.is_some() {
                return Err(rpc_error(
                    ERR_INVALID_PARAMETER,
                    "Range should not be specified for an un-ranged descriptor",
                ));
            }
            push_index(secp, &part, &source, &keymap, None, out)?;
        } else {
            let (begin, end) = range.expect("ranged");
            push_range(secp, &part, &source, &keymap, begin, end, out)?;
        }
    }
    Ok(())
}

fn push_range(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    desc: &Descriptor<DescriptorPublicKey>,
    source: &str,
    keymap: &KeyMap,
    begin: i64,
    end: i64,
    out: &mut Vec<ScanScript>,
) -> Result<(), Value> {
    let mut i = begin;
    while i <= end {
        let idx = u32::try_from(i)
            .map_err(|_| rpc_error(ERR_INVALID_PARAMETER, "End of range is too high"))?;
        push_index(secp, desc, source, keymap, Some(idx), out)?;
        if out.len() > MAX_SCAN_SCRIPTS {
            return Err(rpc_error(
                ERR_INVALID_PARAMETER,
                format!("scan exceeds {MAX_SCAN_SCRIPTS} derived scripts"),
            ));
        }
        i += 1;
    }
    Ok(())
}

fn push_index(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    desc: &Descriptor<DescriptorPublicKey>,
    source: &str,
    keymap: &KeyMap,
    index: Option<u32>,
    out: &mut Vec<ScanScript>,
) -> Result<(), Value> {
    let idx = index.unwrap_or(0);
    if source_has_hardened_wildcard(source) {
        let (script, shown) = hardened_wildcard_script(desc, source, keymap, idx, secp)?;
        out.push(ScanScript {
            script,
            desc: shown,
        });
        return Ok(());
    }
    let definite = desc
        .at_derivation_index(idx)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    let concrete = definite
        .derived_descriptor(secp)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    let shown = core_desc_string(source, index, &concrete, secp)?;
    let spk = concrete.script_pubkey();
    out.push(ScanScript {
        script: spk.as_bytes().to_vec(),
        desc: shown,
    });
    Ok(())
}

/// Core prints `[fingerprint/path]pubkey` with `h` for hardened steps.
fn core_desc_string(
    source: &str,
    index: Option<u32>,
    concrete: &Descriptor<bitcoin::PublicKey>,
    secp: &Secp256k1<bitcoin::secp256k1::All>,
) -> Result<String, Value> {
    let body = strip_checksum(source);
    let pk = pubkey_hex(&strip_checksum(&concrete.to_string()));
    let rewritten = rewrite_extended_key(&body, index, &pk, secp);
    let sum = desc_checksum(&rewritten)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    Ok(format!("{rewritten}#{sum}"))
}

fn strip_checksum(s: &str) -> String {
    s.split('#').next().unwrap_or(s).to_string()
}

fn pubkey_hex(concrete_body: &str) -> String {
    let start = concrete_body.rfind(['0', '1', '2', '3']).unwrap_or(0);
    let bytes = concrete_body.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
        i += 1;
    }
    // Prefer the last long hex run (the pubkey), not a fingerprint.
    let mut best = String::new();
    let b = concrete_body.as_bytes();
    let mut j = 0;
    while j < b.len() {
        if b[j].is_ascii_hexdigit() {
            let s = j;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j - s >= 66 {
                best = concrete_body[s..j].to_string();
            }
        } else {
            j += 1;
        }
    }
    if best.is_empty() {
        concrete_body[start..i].to_string()
    } else {
        best
    }
}

fn rewrite_extended_key(
    body: &str,
    index: Option<u32>,
    pk: &str,
    secp: &Secp256k1<bitcoin::secp256k1::All>,
) -> String {
    let markers = [
        "xprv", "tprv", "yprv", "zprv", "xpub", "tpub", "ypub", "zpub",
    ];
    let Some(start) = markers.iter().filter_map(|m| body.find(m)).min() else {
        return body.to_string();
    };
    let bytes = body.as_bytes();
    let mut end = start;
    while end < bytes.len() && is_base58(bytes[end]) {
        end += 1;
    }
    let key = &body[start..end];
    let mut path_end = end;
    if bytes.get(path_end) == Some(&b'/') {
        path_end += 1;
        while path_end < bytes.len()
            && (bytes[path_end].is_ascii_alphanumeric()
                || matches!(bytes[path_end], b'/' | b'\'' | b'h' | b'*'))
        {
            path_end += 1;
        }
    }
    let path_src = &body[end..path_end];
    let prefix = &body[..start];
    if prefix.ends_with(']') {
        if let Some(br) = prefix.rfind('[') {
            let origin = prefix[br + 1..prefix.len() - 1].replace('\'', "h");
            let hardened = path_src.contains("*h") || path_src.contains("*'");
            let mut extended = format!("[{origin}");
            if let Some(i) = index {
                if !origin.is_empty() && !origin.ends_with('/') {
                    extended.push('/');
                }
                extended.push_str(&i.to_string());
                if hardened {
                    extended.push('h');
                }
            }
            extended.push(']');
            extended.push_str(pk);
            return format!("{}{extended}{}", &prefix[..br], &body[path_end..]);
        }
    }
    let shown = match format_origin_key(key, path_src, index, pk, secp) {
        Some(s) => s,
        None => pk.to_string(),
    };
    format!("{}{shown}{}", prefix, &body[path_end..])
}

fn format_origin_key(
    key: &str,
    path_src: &str,
    index: Option<u32>,
    pk_hex: &str,
    secp: &Secp256k1<bitcoin::secp256k1::All>,
) -> Option<String> {
    let (fp, children) = if key.starts_with("xprv") || key.starts_with("tprv") {
        let xpriv = Xpriv::from_str(key).ok()?;
        let fp = xpriv.fingerprint(secp);
        let mut children = parse_path_steps(path_src);
        if let Some(i) = index {
            let hardened = path_src.contains("*h") || path_src.contains("*'");
            children.push(if hardened {
                ChildNumber::from_hardened_idx(i).ok()?
            } else {
                ChildNumber::from_normal_idx(i).ok()?
            });
        }
        let _ = xpriv;
        (fp, children)
    } else {
        let xpub = Xpub::from_str(key).ok()?;
        let fp = xpub.fingerprint();
        let mut children = parse_path_steps(path_src);
        if let Some(i) = index {
            let hardened = path_src.contains("*h") || path_src.contains("*'");
            children.push(if hardened {
                ChildNumber::from_hardened_idx(i).ok()?
            } else {
                ChildNumber::from_normal_idx(i).ok()?
            });
        }
        (fp, children)
    };
    let mut path = String::new();
    for c in children {
        path.push('/');
        match c {
            ChildNumber::Normal { index } => path.push_str(&index.to_string()),
            ChildNumber::Hardened { index } => {
                path.push_str(&index.to_string());
                path.push('h');
            }
        }
    }
    Some(format!("[{fp}{path}]{pk_hex}"))
}

fn source_has_hardened_wildcard(source: &str) -> bool {
    source.contains("*h") || source.contains("*'")
}

/// miniscript public derivation rejects a hardened wildcard. Apply the xprv path.
fn hardened_wildcard_script(
    desc: &Descriptor<DescriptorPublicKey>,
    source: &str,
    keymap: &KeyMap,
    index: u32,
    secp: &Secp256k1<bitcoin::secp256k1::All>,
) -> Result<(Vec<u8>, String), Value> {
    let body = strip_checksum(source);
    let xprv = keymap.values().find_map(|sk| match sk {
        DescriptorSecretKey::XPrv(x) if x.wildcard == Wildcard::Hardened => Some(x),
        _ => None,
    });
    let Some(xprv) = xprv else {
        return Err(rpc_error(
            ERR_INVALID_PARAMETER,
            format!("Invalid descriptor '{source}'"),
        ));
    };
    let child = ChildNumber::from_hardened_idx(index)
        .map_err(|_| rpc_error(ERR_INVALID_PARAMETER, "End of range is too high"))?;
    let path = xprv.derivation_path.clone().into_child(child);
    let derived = xprv
        .xkey
        .derive_priv(secp, &path)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    let pk = bitcoin::PublicKey::new(derived.private_key.public_key(secp));
    let pk_hex = pk.to_string();
    let spk = script_for_desc(desc, &pk)?;
    let rewritten = rewrite_extended_key(&body, Some(index), &pk_hex, secp);
    let sum = desc_checksum(&rewritten)
        .map_err(|e| rpc_error(ERR_INVALID_PARAMETER, format!("Invalid descriptor ({e})")))?;
    Ok((spk.as_bytes().to_vec(), format!("{rewritten}#{sum}")))
}

fn script_for_desc(
    desc: &Descriptor<DescriptorPublicKey>,
    pk: &bitcoin::PublicKey,
) -> Result<bitcoin::ScriptBuf, Value> {
    let hash = pk
        .wpubkey_hash()
        .map_err(|_| rpc_error(ERR_INVALID_PARAMETER, "Invalid descriptor (uncompressed)"))?;
    match desc {
        Descriptor::Sh(_) => {
            let redeem = bitcoin::ScriptBuf::new_p2wpkh(&hash);
            Ok(redeem.to_p2sh())
        }
        Descriptor::Wpkh(_) => Ok(bitcoin::ScriptBuf::new_p2wpkh(&hash)),
        _ => Ok(bitcoin::ScriptBuf::new_p2pkh(&pk.pubkey_hash())),
    }
}

fn parse_path_steps(path_src: &str) -> Vec<ChildNumber> {
    let mut out = Vec::new();
    for step in path_src
        .split('/')
        .filter(|s| !s.is_empty() && !s.contains('*'))
    {
        let hardened = step.ends_with('h') || step.ends_with('\'');
        let num = step.trim_end_matches(['h', '\'']);
        let Ok(n) = num.parse::<u32>() else {
            continue;
        };
        let child = if hardened {
            ChildNumber::from_hardened_idx(n).ok()
        } else {
            ChildNumber::from_normal_idx(n).ok()
        };
        if let Some(c) = child {
            out.push(c);
        }
    }
    out
}

fn is_base58(b: u8) -> bool {
    matches!(b, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z')
}
