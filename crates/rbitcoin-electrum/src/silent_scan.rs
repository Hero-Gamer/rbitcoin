//! Frigate `blockchain.silentpayments.subscribe` (session-only scan key).

use bitcoin::secp256k1::{PublicKey, SecretKey};
use bitcoin::Network;
use rbitcoin_consensus::{taproot_matches_scan, tweaks_for_height, ChainParams};
use rbitcoin_primitives::{hex_decode, Height};
use rbitcoin_query::Query;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct SpSub {
    pub scan: SecretKey,
    pub spend: PublicKey,
    pub start: u32,
    pub labels: Vec<u32>,
    pub address: String,
}

pub fn parse_sub(params: &Value, network: Network, tip: Option<u32>) -> Result<SpSub, String> {
    let scan_hex = param_str(params, 0)?;
    let spend_hex = param_str(params, 1)?;
    let scan_bytes = hex_decode(scan_hex).map_err(|e| e.to_string())?;
    if scan_bytes.len() != 32 {
        return Err("scan_private_key must be 32 bytes".into());
    }
    let scan = SecretKey::from_slice(&scan_bytes).map_err(|e| e.to_string())?;
    let spend_bytes = hex_decode(spend_hex).map_err(|e| e.to_string())?;
    let spend = PublicKey::from_slice(&spend_bytes).map_err(|e| e.to_string())?;
    let start = match params.as_array().and_then(|a| a.get(2)) {
        None | Some(Value::Null) => 0,
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
        Some(Value::String(s)) if s.contains('-') => s
            .split('-')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    };
    if start > 500_000_000 {
        return Err("timestamp start not supported".into());
    }
    let mut labels = vec![0u32];
    if let Some(arr) = params
        .as_array()
        .and_then(|a| a.get(3))
        .and_then(|v| v.as_array())
    {
        for v in arr {
            if let Some(n) = v.as_u64() {
                let n = n as u32;
                if !labels.contains(&n) {
                    labels.push(n);
                }
            }
        }
    }
    if labels.len() > 10 {
        return Err("too many silent payment labels".into());
    }
    let start = start.min(tip.unwrap_or(start));
    let address = encode_sp_address(network, &scan, &spend);
    Ok(SpSub {
        scan,
        spend,
        start,
        labels,
        address,
    })
}

pub fn subscribe_result(sub: &SpSub) -> Value {
    json!({
        "address": sub.address,
        "labels": sub.labels,
        "start_height": sub.start,
    })
}

pub fn scan_hits(
    query: &Query,
    chain: &ChainParams,
    sub: &SpSub,
    from: u32,
    to: u32,
) -> Result<Vec<Value>, String> {
    let mut hits = Vec::new();
    for h in from..=to {
        let map = tweaks_for_height(query, chain, Height(h)).map_err(|e| e.to_string())?;
        for (txid, tw) in map {
            if tx_matches(sub, &tw) {
                hits.push(json!({
                    "height": h,
                    "tx_hash": rbitcoin_primitives::display_hash_hex(&txid),
                    "tweak_key": rbitcoin_primitives::hex_encode(tw.tweak),
                }));
            }
        }
    }
    Ok(hits)
}

fn tx_matches(sub: &SpSub, tw: &rbitcoin_consensus::TxTweak) -> bool {
    for o in &tw.output_pubkeys {
        for k in 0u32..8 {
            if taproot_matches_scan(&tw.tweak, &o.xonly, &sub.scan, &sub.spend, k) {
                return true;
            }
        }
        for &lab in &sub.labels {
            if lab > 0 && taproot_matches_scan(&tw.tweak, &o.xonly, &sub.scan, &sub.spend, lab) {
                return true;
            }
        }
    }
    false
}

fn encode_sp_address(network: Network, scan_sk: &SecretKey, spend: &PublicKey) -> String {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_pk = PublicKey::from_secret_key(&secp, scan_sk);
    let hrp = match network {
        Network::Bitcoin => "sp",
        Network::Regtest => "sprt",
        Network::Signet | Network::Testnet | Network::Testnet4 => "tsp",
    };
    let mut data = Vec::with_capacity(66);
    data.extend_from_slice(&scan_pk.serialize());
    data.extend_from_slice(&spend.serialize());
    match bitcoin::bech32::encode::<bitcoin::bech32::Bech32m>(
        bitcoin::bech32::Hrp::parse_unchecked(hrp),
        &data,
    ) {
        Ok(s) => s,
        Err(_) => format!("{hrp}1{}", rbitcoin_primitives::hex_encode(&data)),
    }
}

fn param_str(params: &Value, i: usize) -> Result<&str, String> {
    params
        .as_array()
        .and_then(|a| a.get(i))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "expected string".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;
    use serde_json::json;

    #[test]
    fn parse_sub_rejects_short_scan_key() {
        let err = match parse_sub(&json!(["00", "02".repeat(33)]), Network::Regtest, Some(0)) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(err.contains("32 bytes"), "{err}");
        let _ = Secp256k1::new();
    }

    #[test]
    fn parse_sub_labels_start_and_networks() {
        let scan = "0f694e068028a717f8af6b9411f9a133dd3565258714cc226594b34db90c1f2c";
        let spend = "025cc9856d6f8375350e123978daac200c260cb5b5ae83106cab90484dcd8fcf36";
        let sub = parse_sub(
            &json!([scan, spend, "12-20", [0, 1, 1]]),
            Network::Bitcoin,
            Some(100),
        )
        .unwrap();
        assert_eq!(sub.start, 12);
        assert_eq!(sub.labels, vec![0, 1]);
        assert!(sub.address.starts_with("sp1"), "{}", sub.address);
        let tsp = parse_sub(&json!([scan, spend]), Network::Signet, Some(3)).unwrap();
        assert!(tsp.address.starts_with("tsp1"), "{}", tsp.address);
        let ts = match parse_sub(
            &json!([scan, spend, 600_000_000]),
            Network::Regtest,
            Some(0),
        ) {
            Err(e) => e,
            Ok(_) => panic!("timestamp"),
        };
        assert!(ts.contains("timestamp"), "{ts}");
        let many = json!([scan, spend, 0, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]]);
        let too = match parse_sub(&many, Network::Regtest, Some(0)) {
            Err(e) => e,
            Ok(_) => panic!("labels"),
        };
        assert!(too.contains("too many"), "{too}");
        let r = subscribe_result(&sub);
        assert_eq!(r["start_height"], 12);
    }
}
