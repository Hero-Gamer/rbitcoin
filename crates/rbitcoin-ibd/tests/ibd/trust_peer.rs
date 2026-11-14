//! Header-batch acceptance under `--trust-peer`.
use bitcoin::block::Version;
use bitcoin::hashes::Hash;
use rbitcoin_consensus::grind_regtest_pow;
use rbitcoin_ibd::ibd::header_walk::{batch_context_ok, diff_at};
use rbitcoin_ibd::ibd::state::IbdWorkState;
use rbitcoin_log::{capture_logs, take_logs};

fn logs() -> String {
    take_logs()
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn trust_peer_header_batch() {
    let (_dir, hub) = rbitcoin_net::chain::tiny_regtest_hub_labeled("trust-peer-headers");
    hub.ensure_genesis().unwrap();
    let gen = hub.tip_hash().unwrap();
    let genesis = hub.header_of(&gen).unwrap();
    let st = IbdWorkState::new(Vec::new(), Some(gen), Some(0));
    let diff = diff_at(&st, &hub, gen);

    let good = rbitcoin_consensus::mine_empty_regtest(gen, genesis.time + 600, 1);
    assert!(
        batch_context_ok(&st, &hub, &[good.header], &diff),
        "a valid header is accepted with the flag off"
    );

    let mut low = good.header;
    low.version = Version::from_consensus(1);
    grind_regtest_pow(&mut low);
    capture_logs(true);
    assert!(
        !batch_context_ok(&st, &hub, &[low], &diff),
        "flag off rejects an outdated version"
    );
    let off = logs();
    assert!(
        !off.contains("trust-peer:"),
        "flag off header reject has no trust-peer line: {off}"
    );
    assert!(
        hub.ensure_headers_batch(&[low]).is_err(),
        "flag off does not store the header"
    );
    assert!(hub.header_of(&low.block_hash()).is_none());

    hub.query.set_trust_peer(true);
    capture_logs(false);
    capture_logs(true);
    assert!(
        batch_context_ok(&st, &hub, &[low], &diff),
        "trust-peer keeps a header that fails nVersion"
    );
    let stored = hub
        .ensure_headers_batch(&[low])
        .expect("trust-peer stores the header on its prev link");
    assert_eq!(stored.len(), 1);
    assert!(hub.header_of(&low.block_hash()).is_some());
    let on = logs();
    let hash = low.block_hash();
    let needle = format!("trust-peer: applied block {hash} height 1:");
    assert!(on.contains(&needle), "missing {needle} in {on}");
    assert!(
        on.contains("bad-version"),
        "version fault keeps its reject reason: {on}"
    );
    assert_eq!(
        on.matches(&needle).count(),
        1,
        "walk and store must not double-log: {on}"
    );

    let mut orphan = rbitcoin_consensus::mine_empty_regtest(low.block_hash(), low.time + 600, 2);
    orphan.header.prev_blockhash = gen;
    capture_logs(true);
    assert!(
        !batch_context_ok(&st, &hub, &[low, orphan.header], &diff),
        "a wrong prev is not applied"
    );
    let stopped = logs();
    assert!(
        stopped.contains("trust-peer: not applied"),
        "wrong prev logs not applied: {stopped}"
    );
    assert!(
        stopped.contains("unexpected previous header"),
        "wrong prev keeps its reason: {stopped}"
    );
    capture_logs(false);
}

/// A header whose parent is stored but not confirmed still runs proof-of-work.
/// A miss is not stored, flag on or off. Height 1's parent is genesis, which
/// is on the tip index. This is the next one.
#[test]
fn trust_peer_off_tip_header_high_hash_is_not_stored() {
    let (_dir, hub) = rbitcoin_net::chain::tiny_regtest_hub_labeled("trust-peer-off-tip");
    hub.ensure_genesis().unwrap();
    let gen = hub.tip_hash().unwrap();
    let genesis = hub.header_of(&gen).unwrap();
    let good = rbitcoin_consensus::mine_empty_regtest(gen, genesis.time + 600, 1);
    hub.ensure_headers_batch(&[good.header])
        .expect("valid header stores ahead of the tip");
    let parent = good.header.block_hash();
    assert!(
        hub.query
            .height_of_hash(&parent.to_byte_array())
            .unwrap()
            .is_none(),
        "the parent is not on the confirmed height index"
    );
    let mut bad = rbitcoin_consensus::mine_empty_regtest(parent, good.header.time + 600, 2);
    miss_regtest_pow(&mut bad.header);

    capture_logs(false);
    capture_logs(true);
    assert!(
        hub.ensure_headers_batch(&[bad.header]).is_err(),
        "flag off does not store a high-hash header"
    );
    assert!(hub.header_of(&bad.header.block_hash()).is_none());
    let off = logs();
    assert!(!off.contains("trust-peer:"), "{off}");

    hub.query.set_trust_peer(true);
    capture_logs(false);
    capture_logs(true);
    assert!(
        hub.ensure_headers_batch(&[bad.header]).is_err(),
        "trust-peer does not store a high-hash header"
    );
    assert!(hub.header_of(&bad.header.block_hash()).is_none());
    let on = logs();
    let hash = bad.header.block_hash();
    let needle = format!("trust-peer: not applied block {hash} height 2:");
    assert!(on.contains(&needle), "{on}");
    assert!(on.contains("high-hash"), "{on}");
    assert!(!on.contains("trust-peer: applied"), "{on}");
    capture_logs(false);
}

/// Unexpected nBits whose hash still meets that target is not stored. Claimed
/// work stays put, and the log is the bits reject.
#[test]
fn trust_peer_off_tip_header_unexpected_bits_is_not_stored() {
    let (_dir, hub) = rbitcoin_net::chain::tiny_regtest_hub_labeled("trust-peer-bits");
    hub.ensure_genesis().unwrap();
    let gen = hub.tip_hash().unwrap();
    let genesis = hub.header_of(&gen).unwrap();
    let good = rbitcoin_consensus::mine_empty_regtest(gen, genesis.time + 600, 1);
    hub.ensure_headers_batch(&[good.header])
        .expect("valid header stores ahead of the tip");
    let mut bad =
        rbitcoin_consensus::mine_empty_regtest(good.header.block_hash(), good.header.time + 600, 2);
    bad.header.bits = bitcoin::CompactTarget::from_consensus(0x1f7f_ffff);
    bad.header.nonce = 0;
    let limit = rbitcoin_consensus::ChainParams::regtest().pow_limit;
    let mut met = false;
    for _ in 0..200_000 {
        if rbitcoin_consensus::header::pow_hash_meets_target(
            bad.header.block_hash().to_byte_array(),
            bad.header.bits,
            limit,
        )
        .is_ok()
        {
            met = true;
            break;
        }
        bad.header.nonce = bad.header.nonce.wrapping_add(1);
    }
    assert!(met, "fixture hash must meet the unexpected target");
    assert_ne!(bad.header.bits, good.header.bits);
    let before = hub.chain_work().unwrap();
    hub.query.set_trust_peer(true);
    capture_logs(false);
    capture_logs(true);
    assert!(
        hub.ensure_headers_batch(&[bad.header]).is_err(),
        "trust-peer stored a header with unexpected nBits"
    );
    assert!(hub.header_of(&bad.header.block_hash()).is_none());
    assert_eq!(hub.chain_work().unwrap(), before);
    let on = logs();
    let hash = bad.header.block_hash();
    let needle = format!("trust-peer: not applied block {hash} height 2:");
    assert!(on.contains(&needle), "{on}");
    assert!(on.contains("bad-diffbits"), "{on}");
    assert!(!on.contains("trust-peer: applied"), "{on}");
    capture_logs(false);
}

fn miss_regtest_pow(header: &mut bitcoin::block::Header) {
    let bits = header.bits;
    let limit = rbitcoin_consensus::ChainParams::regtest().pow_limit;
    for _ in 0..64 {
        header.nonce = header.nonce.wrapping_add(1);
        match rbitcoin_consensus::header::pow_hash_meets_target(
            header.block_hash().to_byte_array(),
            bits,
            limit,
        ) {
            Err(rbitcoin_consensus::ConsensusError::InvalidPow) => return,
            Ok(()) => {}
            Err(err) => panic!("expected high-hash, got {err}"),
        }
    }
    panic!("regtest target accepted every nonce in the walk");
}

/// Gossip is not a dial candidate while `--trust-peer` is set.
#[test]
fn trust_peer_ignores_learned_addrs() {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    let mut book = rbitcoin_net::AddrMan::new();
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8333);
    let learned =
        rbitcoin_net::NetAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 8333));
    rbitcoin_ibd::ibd::events::learn_peer_addrs(
        true,
        &mut book,
        &[learned],
        local,
        rbitcoin_net::NetAddr::Ip(local),
        0,
    );
    assert!(book.is_empty(), "trust-peer must not keep gossip");
    rbitcoin_ibd::ibd::events::learn_peer_addrs(
        false,
        &mut book,
        &[learned],
        local,
        rbitcoin_net::NetAddr::Ip(local),
        0,
    );
    assert_eq!(book.len(), 1, "flag off still learns addresses");
}
