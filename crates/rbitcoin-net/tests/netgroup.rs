use rbitcoin_net::asmap::two_prefix_asmap_bytes;
use rbitcoin_net::asmap::AsMap;
use rbitcoin_net::netgroup::*;
use std::net::{IpAddr, SocketAddr};
use std::net::{Ipv4Addr, Ipv6Addr};

fn v4(a: u8, b: u8, c: u8, d: u8) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), 8333)
}

#[test]
fn netgroup_prefix_ipv4_slash16() {
    assert_eq!(
        netgroup(v4(1, 2, 3, 4), None),
        netgroup(v4(1, 2, 9, 9), None)
    );
    assert_ne!(
        netgroup(v4(1, 2, 3, 4), None),
        netgroup(v4(1, 3, 0, 1), None)
    );
}

#[test]
fn netgroup_prefix_ipv6_slash32() {
    let a = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        1,
    );
    let b = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 0, 0, 0, 0, 1)),
        1,
    );
    let c = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb9, 0, 0, 0, 0, 0, 1)),
        1,
    );
    assert_eq!(netgroup(a, None), netgroup(b, None));
    assert_ne!(netgroup(a, None), netgroup(c, None));
}

fn v6(s: &str) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(s.parse().unwrap()), 8333)
}

/// Core `GetGroup` / `GetLinkedIPv4`: IPv4-mapped, NAT64 (RFC 6052), SIIT
/// (RFC 6145), 6to4 (RFC 3964) and Teredo (RFC 4380, bit-flipped) take the
/// /16 of the IPv4 they carry.
#[test]
fn netgroup_embedded_ipv4_takes_its_slash16() {
    let g = netgroup(v4(45, 1, 9, 9), None);
    for embedded in [
        "::ffff:45.1.0.2",
        "64:ff9b::45.1.0.2",
        "::ffff:0:45.1.0.2",
        "2002:2d01:2::",
        "2001:0:4136:e378:8000:63bf:d2fe:fffd",
    ] {
        assert_eq!(netgroup(v6(embedded), None), g, "{embedded}");
    }
    assert_ne!(netgroup(v6("2002:2d02:2::"), None), g);
}

/// Core looks a linked IPv4 up in the asmap as that IPv4.
#[test]
fn netgroup_asmap_maps_embedded_ipv4_as_ipv4() {
    let m = AsMap::from_bytes(two_prefix_asmap_bytes()).unwrap();
    assert_eq!(
        netgroup(v6("2002:102:304::"), Some(&m)),
        netgroup(v4(1, 2, 9, 9), Some(&m))
    );
}

/// Core groups Hurricane Electric `2001:470::/32` by /36, the rest of IPv6
/// by /32.
#[test]
fn netgroup_henet_slash36() {
    let a = netgroup(v6("2001:470:1000::1"), None);
    assert_eq!(a, netgroup(v6("2001:470:1fff::1"), None));
    assert_ne!(a, netgroup(v6("2001:470:2000::1"), None));
    assert_ne!(a, netgroup(v6("2001:470::1"), None));
    assert_eq!(
        netgroup(v6("2001:471:1000::1"), None),
        netgroup(v6("2001:471:2000::1"), None)
    );
}

/// Core keys CJDNS by the four bits after the constant `fc` byte, as Tor
/// and I2P by their first four bits, not by /32.
#[test]
fn netgroup_cjdns_four_bits_after_fc() {
    let a = netgroup(v6("fc10::1"), None);
    assert_eq!(a, netgroup(v6("fc1f:ffff::1"), None));
    assert_ne!(a, netgroup(v6("fc20::1"), None));
}

/// Core's leading network-class byte keeps IPv4 and IPv6 groups apart. An
/// IPv6 /32 whose first sixteen bits are zero is not an IPv4 /16.
#[test]
fn netgroup_ipv6_never_shares_an_ipv4_key() {
    for (ip6, ip4) in [
        ("0:2d01::1", v4(45, 1, 0, 1)),
        ("0:2d0::1", v4(45, 0, 0, 1)),
    ] {
        assert_ne!(netgroup(v6(ip6), None), netgroup(ip4, None), "{ip6}");
    }
}

#[test]
fn netgroup_asmap_two_asns() {
    let m = AsMap::from_bytes(two_prefix_asmap_bytes()).unwrap();
    let a = v4(1, 2, 3, 4);
    let b = v4(1, 2, 9, 9);
    let c = v4(3, 4, 0, 1);
    assert_eq!(netgroup(a, Some(&m)), netgroup(b, Some(&m)));
    assert_ne!(netgroup(a, Some(&m)), netgroup(c, Some(&m)));
    let v6 = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0102, 0x0304)),
        1,
    );
    assert_eq!(netgroup(a, Some(&m)), netgroup(v6, Some(&m)));
}

/// With asmap, an address the map does not cover (ASN 0) keeps its own
/// prefix group, as Core's `GetGroup` does: unmapped addresses do not all
/// share one group. An ASN key never equals a prefix group.
#[test]
fn netgroup_asmap_unmapped_falls_back_to_prefix() {
    let m = AsMap::from_bytes(two_prefix_asmap_bytes()).unwrap();
    assert_eq!(m.mapped_as(v4(5, 6, 0, 1).ip()), 0);
    assert_ne!(
        netgroup(v4(5, 6, 0, 1), Some(&m)),
        netgroup(v4(5, 7, 0, 1), Some(&m))
    );
    assert_eq!(
        netgroup(v4(5, 6, 0, 1), Some(&m)),
        netgroup(v4(5, 6, 0, 1), None)
    );
    assert_eq!(m.mapped_as(v4(1, 2, 3, 4).ip()), 1);
    assert_ne!(
        netgroup(v4(1, 2, 3, 4), Some(&m)),
        netgroup(v4(0, 1, 0, 1), None),
        "ASN 1 and the 0.1/16 prefix group are different groups"
    );
}

/// Core leaves Tor, I2P and CJDNS out of the outbound group rule, and never
/// dials a local or unroutable address automatically. Those rows are exempt.
#[test]
fn outbound_group_exempts_cjdns_and_unroutable() {
    let cjdns: SocketAddr = "[fc00::1]:8333".parse().unwrap();
    let ula: SocketAddr = "[fd00::1]:8333".parse().unwrap();
    for exempt in [
        v4(127, 0, 0, 1),
        v4(10, 0, 0, 1),
        v4(192, 168, 1, 1),
        cjdns,
        ula,
    ] {
        assert_eq!(outbound_group(exempt, None), None, "{exempt}");
    }
    assert_eq!(
        outbound_group(v4(1, 2, 3, 4), None),
        Some(netgroup(v4(1, 2, 3, 4), None))
    );
}

fn group_octet0(a: SocketAddr) -> Option<u64> {
    match a.ip() {
        IpAddr::V4(v) => Some(u64::from(v.octets()[0])),
        IpAddr::V6(_) => None,
    }
}

#[test]
fn select_diverse_skips_occupied_asn() {
    use std::collections::HashSet;
    let m = AsMap::from_bytes(two_prefix_asmap_bytes()).unwrap();
    let ranked = vec![v4(1, 2, 0, 1), v4(3, 4, 0, 1), v4(1, 2, 0, 2)];
    let mut occupied = HashSet::new();
    occupied.insert(netgroup(v4(1, 2, 0, 9), Some(&m)));
    let got = select_diverse(&ranked, 2, &occupied, |a| Some(netgroup(a, Some(&m))));
    assert_eq!(got, vec![v4(3, 4, 0, 1)]);
}

#[test]
fn select_diverse_skips_occupied() {
    use std::collections::HashSet;
    let ranked: Vec<SocketAddr> = (1..=4).map(|i| v4(i, 0, 0, 1)).collect();
    let mut occupied = HashSet::new();
    occupied.insert(1);
    let got = select_diverse(&ranked, 3, &occupied, group_octet0);
    assert_eq!(got, vec![v4(2, 0, 0, 1), v4(3, 0, 0, 1), v4(4, 0, 0, 1)]);
}

/// Core opens at most one automatic outbound per group. A book in one group
/// yields one pick, and a group a live outbound peer holds yields none.
#[test]
fn select_diverse_thin_book_takes_one_per_group() {
    use std::collections::HashSet;
    let ranked = vec![v4(1, 0, 0, 1), v4(1, 0, 0, 2), v4(1, 0, 0, 3)];
    let got = select_diverse(&ranked, 3, &HashSet::new(), group_octet0);
    assert_eq!(got, vec![v4(1, 0, 0, 1)]);
    let occupied = HashSet::from([1]);
    assert!(select_diverse(&ranked, 3, &occupied, group_octet0).is_empty());
}

#[test]
fn select_diverse_two_groups_eight_slots_takes_two() {
    use std::collections::HashSet;
    let mut ranked = Vec::new();
    for host in 1u8..=8 {
        ranked.push(v4(44, 0, 0, host));
        ranked.push(v4(45, 0, 0, host));
    }
    let got = select_diverse(&ranked, 8, &HashSet::new(), group_octet0);
    assert_eq!(got, vec![v4(44, 0, 0, 1), v4(45, 0, 0, 1)]);
}

/// A row whose group is `None` (exempt) is taken whatever the groups.
#[test]
fn select_diverse_takes_every_exempt_row() {
    use std::collections::HashSet;
    let ranked: Vec<SocketAddr> = (1..=4)
        .map(|p| SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), p))
        .collect();
    let got = select_diverse(&ranked, 8, &HashSet::new(), group_octet0);
    assert_eq!(got, ranked);
}

/// Onion and I2P get their own groups, keyed by network and the first four
/// address bits as Core does. Neither shares the `0.0.0.0` placeholder group.
#[test]
fn netgroup_net_keys_overlay_by_network() {
    use rbitcoin_net::NetAddr;
    let onion = |b: u8| NetAddr::Onion {
        pk: [b; 32],
        port: 8333,
    };
    let i2p = NetAddr::I2p {
        dest: [0x10; 32],
        port: 8333,
    };
    let placeholder = netgroup(SocketAddr::from(([0, 0, 0, 0], 8333)), None);
    assert_eq!(
        netgroup_net(onion(0x10), None),
        netgroup_net(onion(0x1f), None),
        "same first four bits share a group"
    );
    assert_ne!(
        netgroup_net(onion(0x10), None),
        netgroup_net(onion(0x20), None)
    );
    assert_ne!(netgroup_net(onion(0x10), None), netgroup_net(i2p, None));
    assert_ne!(netgroup_net(onion(0x00), None), placeholder);
    assert_ne!(netgroup_net(i2p, None), placeholder);
    assert_eq!(
        netgroup_net(NetAddr::Ip(v4(1, 2, 3, 4)), None),
        netgroup(v4(1, 2, 3, 4), None)
    );
}
