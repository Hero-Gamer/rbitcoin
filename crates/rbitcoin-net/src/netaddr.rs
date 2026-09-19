//! BIP155-capable peer identity: clearnet [`SocketAddr`] and Tor v3 onion.

use crate::error::NetError;
use sha3::{Digest, Sha3_256};
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const ONION_VERSION: u8 = 3;
const ONION_NAME_LEN: usize = 56;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetAddr {
    Ip(SocketAddr),
    Onion { pk: [u8; 32], port: u16 },
}

impl fmt::Display for NetAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            NetAddr::Ip(addr) => write!(f, "{addr}"),
            NetAddr::Onion { pk, port } => {
                write!(f, "{}.onion:{port}", encode_onion_name(&pk))
            }
        }
    }
}

impl FromStr for NetAddr {
    type Err = NetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_net_addr(s)
    }
}

fn parse_net_addr(s: &str) -> Result<NetAddr, NetError> {
    let Some((host, port_s)) = s.rsplit_once(':') else {
        return Err(NetError::Encode(format!("bad peer address {s}")));
    };
    if let Some(name) = strip_onion_suffix(host) {
        let port: u16 = port_s
            .parse()
            .map_err(|_| NetError::Encode(format!("bad onion port {s}")))?;
        let pk = decode_onion_name(name)
            .ok_or_else(|| NetError::Encode(format!("bad onion address {s}")))?;
        return Ok(NetAddr::Onion { pk, port });
    }
    s.parse()
        .map(NetAddr::Ip)
        .map_err(|_| NetError::Encode(format!("bad peer address {s}")))
}

fn strip_onion_suffix(host: &str) -> Option<&str> {
    let b = host.as_bytes();
    if b.len() < 6 {
        return None;
    }
    if !b[b.len() - 6..].eq_ignore_ascii_case(b".onion") {
        return None;
    }
    Some(&host[..host.len() - 6])
}

fn onion_checksum(pk: &[u8; 32]) -> [u8; 2] {
    let mut h = Sha3_256::new();
    h.update(b".onion checksum");
    h.update(pk);
    h.update([ONION_VERSION]);
    let d = h.finalize();
    [d[0], d[1]]
}

fn encode_onion_name(pk: &[u8; 32]) -> String {
    let mut payload = [0u8; 35];
    payload[..32].copy_from_slice(pk);
    let sum = onion_checksum(pk);
    payload[32..34].copy_from_slice(&sum);
    payload[34] = ONION_VERSION;
    b32_encode(&payload)
}

fn decode_onion_name(name: &str) -> Option<[u8; 32]> {
    if name.len() != ONION_NAME_LEN {
        return None;
    }
    let payload = b32_decode(name)?;
    if payload[34] != ONION_VERSION {
        return None;
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&payload[..32]);
    if onion_checksum(&pk) != [payload[32], payload[33]] {
        return None;
    }
    Some(pk)
}

fn b32_encode(bytes: &[u8; 35]) -> String {
    let mut out = String::with_capacity(ONION_NAME_LEN);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    out
}

fn b32_decode(s: &str) -> Option<[u8; 35]> {
    let mut out = [0u8; 35];
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut n = 0usize;
    for c in s.bytes() {
        let v = match c {
            b'a'..=b'z' => c - b'a',
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => 26 + (c - b'2'),
            _ => return None,
        };
        acc = (acc << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if n >= 35 {
                return None;
            }
            out[n] = (acc >> bits) as u8;
            n += 1;
        }
    }
    if n != 35 || bits != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn netaddr_onion_parse_roundtrip() {
        let s = "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333";
        let a: NetAddr = s.parse().expect("valid v3 onion");
        assert_eq!(a.to_string(), s);
        assert_eq!(a, s.parse().unwrap());
        match a {
            NetAddr::Onion { pk, port } => {
                assert_eq!(
                    pk,
                    [
                        0x79, 0xbc, 0xc6, 0x25, 0x18, 0x4b, 0x05, 0x19, 0x49, 0x75, 0xc2, 0x8b,
                        0x66, 0xb6, 0x6b, 0x04, 0x69, 0xf7, 0xf6, 0x55, 0x6f, 0xb1, 0xac, 0x31,
                        0x89, 0xa7, 0x9b, 0x40, 0xdd, 0xa3, 0x2f, 0x1f,
                    ]
                );
                assert_eq!(port, 8333);
            }
            NetAddr::Ip(_) => panic!("expected onion"),
        }
        assert!(
            "qg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333"
                .parse::<NetAddr>()
                .is_err()
        );
        assert!(
            "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscry.onion:8333"
                .parse::<NetAddr>()
                .is_err()
        );
        assert!("short.onion:8333".parse::<NetAddr>().is_err());
        assert!("not-an-addr".parse::<NetAddr>().is_err());
    }

    #[test]
    fn netaddr_ip_parse_roundtrip() {
        let s = "1.2.3.4:8333";
        let a: NetAddr = s.parse().expect("ipv4");
        assert_eq!(
            a,
            NetAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 8333))
        );
        assert_eq!(a.to_string(), s);
    }
}
