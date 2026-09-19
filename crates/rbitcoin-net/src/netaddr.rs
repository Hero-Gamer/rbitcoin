//! BIP155-capable peer identity: clearnet [`SocketAddr`] and Tor v3 onion.

use crate::error::NetError;
use bitcoin::p2p::address::{AddrV2, AddrV2Message};
use sha3::{Digest, Sha3_256};
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const ONION_VERSION: u8 = 3;
const ONION_NAME_LEN: usize = 56;
const I2P_B32_LEN: usize = 52;
const I2P_SUFFIX: &str = ".b32.i2p";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnlyNet {
    Ipv4,
    Ipv6,
    Onion,
}

impl OnlyNet {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ipv4" => Ok(Self::Ipv4),
            "ipv6" => Ok(Self::Ipv6),
            "onion" => Ok(Self::Onion),
            "i2p" | "cjdns" => Err(format!("unknown network {s} (not yet implemented)")),
            other => Err(format!("unknown network {other}")),
        }
    }

    pub fn matches_addr(self, addr: NetAddr) -> bool {
        match (self, addr) {
            (Self::Ipv4, NetAddr::Ip(s)) => s.is_ipv4(),
            (Self::Ipv6, NetAddr::Ip(s)) => s.is_ipv6(),
            (Self::Onion, NetAddr::Onion { .. }) => true,
            _ => false,
        }
    }
}

pub fn addr_allowed(addr: NetAddr, only: &[OnlyNet]) -> bool {
    only.is_empty() || only.iter().any(|n| n.matches_addr(addr))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetAddr {
    Ip(SocketAddr),
    Onion { pk: [u8; 32], port: u16 },
    I2p { dest: [u8; 32], port: u16 },
}

impl fmt::Display for NetAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            NetAddr::Ip(addr) => write!(f, "{addr}"),
            NetAddr::Onion { pk, port } => {
                write!(f, "{}.onion:{port}", encode_onion_name(&pk))
            }
            NetAddr::I2p { dest, port } => {
                write!(f, "{}{I2P_SUFFIX}:{port}", encode_i2p_name(&dest))
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

impl NetAddr {
    pub fn from_addrv2(msg: &AddrV2Message) -> Option<Self> {
        if msg.port == 0 {
            return None;
        }
        match &msg.addr {
            AddrV2::Ipv4(_) | AddrV2::Ipv6(_) => msg.socket_addr().ok().map(NetAddr::Ip),
            AddrV2::TorV3(pk) => Some(NetAddr::Onion {
                pk: *pk,
                port: msg.port,
            }),
            AddrV2::I2p(dest) => Some(NetAddr::I2p {
                dest: *dest,
                port: msg.port,
            }),
            AddrV2::TorV2(_) | AddrV2::Cjdns(_) | AddrV2::Unknown(_, _) => None,
        }
    }

    pub fn socket_addr(self) -> Option<SocketAddr> {
        match self {
            NetAddr::Ip(s) => Some(s),
            NetAddr::Onion { .. } | NetAddr::I2p { .. } => None,
        }
    }

    pub fn is_ipv6(self) -> bool {
        match self {
            NetAddr::Ip(s) => s.is_ipv6(),
            NetAddr::Onion { .. } | NetAddr::I2p { .. } => false,
        }
    }

    pub fn port(self) -> u16 {
        match self {
            NetAddr::Ip(s) => s.port(),
            NetAddr::Onion { port, .. } | NetAddr::I2p { port, .. } => port,
        }
    }

    pub fn host_str(self) -> String {
        match self {
            NetAddr::Ip(s) => s.ip().to_string(),
            NetAddr::Onion { pk, .. } => format!("{}.onion", encode_onion_name(&pk)),
            NetAddr::I2p { dest, .. } => format!("{}{I2P_SUFFIX}", encode_i2p_name(&dest)),
        }
    }

    pub fn network_label(self) -> &'static str {
        match self {
            NetAddr::Ip(s) if s.is_ipv4() => "ipv4",
            NetAddr::Ip(_) => "ipv6",
            NetAddr::Onion { .. } => "onion",
            NetAddr::I2p { .. } => "i2p",
        }
    }
}

fn parse_net_addr(s: &str) -> Result<NetAddr, NetError> {
    let Some((host, port_s)) = s.rsplit_once(':') else {
        return Err(NetError::Encode(format!("bad peer address {s}")));
    };
    if let Some(name) = strip_i2p_suffix(host) {
        let port: u16 = port_s
            .parse()
            .map_err(|_| NetError::Encode(format!("bad i2p port {s}")))?;
        let dest = decode_i2p_name(name)
            .ok_or_else(|| NetError::Encode(format!("bad i2p address {s}")))?;
        return Ok(NetAddr::I2p { dest, port });
    }
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

fn strip_i2p_suffix(host: &str) -> Option<&str> {
    let b = host.as_bytes();
    let suf = I2P_SUFFIX.as_bytes();
    if b.len() <= suf.len() {
        return None;
    }
    if !b[b.len() - suf.len()..].eq_ignore_ascii_case(suf) {
        return None;
    }
    Some(&host[..host.len() - suf.len()])
}

fn encode_i2p_name(dest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(I2P_B32_LEN);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &b in dest {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn decode_i2p_name(name: &str) -> Option<[u8; 32]> {
    if name.len() != I2P_B32_LEN {
        return None;
    }
    let mut out = [0u8; 32];
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut n = 0usize;
    for c in name.bytes() {
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
            if n >= 32 {
                return None;
            }
            out[n] = (acc >> bits) as u8;
            n += 1;
        }
    }
    if n != 32 {
        return None;
    }
    Some(out)
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
            NetAddr::Ip(_) | NetAddr::I2p { .. } => panic!("expected onion"),
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

    #[test]
    fn netaddr_i2p_addrv2_roundtrip() {
        let dest = [0x11u8; 32];
        let msg = AddrV2Message {
            time: 1,
            services: bitcoin::p2p::ServiceFlags::NETWORK,
            addr: AddrV2::I2p(dest),
            port: 8333,
        };
        let a = NetAddr::from_addrv2(&msg).expect("i2p addrv2");
        assert_eq!(a, NetAddr::I2p { dest, port: 8333 });
        let s = a.to_string();
        assert!(s.ends_with(".b32.i2p:8333"), "{s}");
        assert_eq!(s.parse::<NetAddr>().unwrap(), a);
        assert_eq!(a.network_label(), "i2p");
        assert_eq!(a.port(), 8333);
        assert!(a.socket_addr().is_none());
        let zeros = NetAddr::I2p {
            dest: [0u8; 32],
            port: 1,
        };
        assert_eq!(
            zeros.to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.b32.i2p:1"
        );
        assert!("short.b32.i2p:1".parse::<NetAddr>().is_err());
    }
}
