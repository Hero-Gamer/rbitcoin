//! Real BIP324 → body queue → full-validation confirmation, without external peers.
use bitcoin::hashes::Hash;
use bitcoin::p2p::{message::NetworkMessage, message_blockdata::Inventory, Magic};
use bitcoin::Block;
use rbitcoin_consensus::{ChainParams, Milestone};
use rbitcoin_ibd::ibd::IbdConfig;
use rbitcoin_net::peer::{connect_and_handshake_timed, HandshakePolicy};
use rbitcoin_net::service::P2PNode;
use rbitcoin_net::v2::{read_v2_frame, write_v2_msg};
use rbitcoin_query::Query;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug)]
enum Replay {
    ResponsiveHeaders,
    SilentHeaders,
    InitialPeerEof,
}

#[derive(Default)]
struct ReplayStats {
    header_requests: AtomicUsize,
    frontier_requests: AtomicUsize,
    frontier_failures: AtomicUsize,
}

pub(super) fn chain(count: u32) -> Vec<Block> {
    let mut blocks = vec![bitcoin::blockdata::constants::genesis_block(
        bitcoin::Network::Regtest,
    )];
    for height in 1..=count {
        let previous = blocks.last().unwrap();
        blocks.push(rbitcoin_consensus::mine_empty_regtest(
            previous.block_hash(),
            previous.header.time + 600,
            height,
        ));
    }
    blocks
}

async fn serve_delayed_frontier(
    listener: &TcpListener,
    blocks: Arc<Vec<Block>>,
    mut release: oneshot::Receiver<()>,
    mode: Replay,
    stats: Arc<ReplayStats>,
) -> Result<(), rbitcoin_net::error::NetError> {
    let (stream, remote) = listener.accept().await?;
    let local = stream.local_addr()?;
    let (_, mut reader, mut writer, _, _shutdown) = connect_and_handshake_timed(
        Duration::from_secs(5),
        stream,
        Magic::REGTEST,
        local,
        remote,
        64,
        true,
        "/frontier-replay/",
        HandshakePolicy::plain(),
    )
    .await?;
    let mut released = false;
    let mut frontier_requested = false;
    let mut pending: VecDeque<usize> = VecDeque::new();
    let mut sends = tokio::time::interval(Duration::from_millis(5));
    loop {
        let frame = tokio::select! {
            _ = sends.tick(), if !pending.is_empty() => {
                let height = pending.pop_front().unwrap();
                write_v2_msg(&mut writer, NetworkMessage::Block(blocks[height].clone())).await?;
                continue;
            }
            result = &mut release, if !released => {
                if result.is_err() { return Ok(()); }
                released = true;
                assert!(frontier_requested, "release requires an actual getdata for block 1");
                write_v2_msg(&mut writer, NetworkMessage::Block(blocks[1].clone())).await?;
                continue;
            }
            result = read_v2_frame(&mut reader, Magic::REGTEST) => result?,
        };
        let message = rbitcoin_net::msg_decode::decode_framed_offload(frame).await?;
        match message.payload() {
            NetworkMessage::GetHeaders(request) => {
                let previous = stats.header_requests.fetch_add(1, Ordering::SeqCst);
                if previous > 0 && matches!(mode, Replay::SilentHeaders) {
                    continue;
                }
                let start = blocks
                    .iter()
                    .rposition(|block| request.locator_hashes.contains(&block.block_hash()))
                    .map_or(1, |height| height + 1);
                let headers = blocks[start..].iter().map(|block| block.header).collect();
                write_v2_msg(&mut writer, NetworkMessage::Headers(headers)).await?;
            }
            NetworkMessage::GetData(inventory) => {
                for item in inventory {
                    let (Inventory::Block(hash) | Inventory::WitnessBlock(hash)) = item else {
                        continue;
                    };
                    let Some(height) = blocks.iter().position(|block| block.block_hash() == *hash)
                    else {
                        continue;
                    };
                    if height == 1 {
                        stats.frontier_requests.fetch_add(1, Ordering::SeqCst);
                        if matches!(mode, Replay::InitialPeerEof) {
                            stats.frontier_failures.fetch_add(1, Ordering::SeqCst);
                            return Ok(());
                        }
                        frontier_requested = true;
                        if !released {
                            continue;
                        }
                    }
                    pending.push_back(height);
                }
            }
            NetworkMessage::Ping(nonce) => {
                write_v2_msg(&mut writer, NetworkMessage::Pong(*nonce)).await?
            }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn later_bodies_wait_for_frontier_then_fully_validate() {
    run_replay(Replay::ResponsiveHeaders).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in BIP324 replay of real header deadlines; default coverage backdates the header lane"]
async fn header_timeouts_preserve_frontier_delivery_from_productive_peer() {
    run_replay(Replay::SilentHeaders).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frontier_is_reassigned_after_initial_peer_eof_then_validated() {
    run_replay(Replay::InitialPeerEof).await;
}

async fn run_replay(mode: Replay) {
    let dir = rbitcoin_store::testutil::TempDir::labeled("frontier-replay").unwrap();
    let node = P2PNode::start(
        "127.0.0.1:0".parse().unwrap(),
        Query::open_or_create_tiny(dir.join("store")).unwrap(),
        ChainParams::regtest(),
        Milestone::NONE,
        rbitcoin_net::REDIAL_INTERVAL_TEST,
    )
    .await
    .unwrap();
    let blocks = Arc::new(chain(6));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = rbitcoin_net::NetAddr::Ip(listener.local_addr().unwrap());
    let (release, released) = oneshot::channel();
    let stats = Arc::new(ReplayStats::default());
    let server_blocks = blocks.clone();
    let server_stats = stats.clone();
    let server = tokio::spawn(async move {
        let healthy_mode = if matches!(mode, Replay::InitialPeerEof) {
            let (_hold, first_release) = oneshot::channel();
            serve_delayed_frontier(
                &listener,
                server_blocks.clone(),
                first_release,
                mode,
                server_stats.clone(),
            )
            .await?;
            Replay::ResponsiveHeaders
        } else {
            mode
        };
        serve_delayed_frontier(
            &listener,
            server_blocks,
            released,
            healthy_mode,
            server_stats,
        )
        .await
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let mut cfg = IbdConfig::for_test();
    cfg.target_peers = 1;
    cfg.window = 32;
    cfg.per_peer = 16;
    cfg.stall = Duration::from_secs(60);
    cfg.connect_timeout = Duration::from_secs(5);
    let addresses = [address];
    if matches!(mode, Replay::InitialPeerEof) {
        cfg.initial_dial_waves = 1;
        cfg.peerless = rbitcoin_ibd::Peerless::Redial {
            cap: Duration::from_millis(100),
        };
    }
    let sync = rbitcoin_ibd::sync_cancellable(&node, &addresses, cfg, Some(cancel.clone()));
    let control = async {
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            while !blocks[2..].iter().all(|block| {
                node.query
                    .block_queue_has_hash(&block.block_hash().to_byte_array())
            }) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                node.tip_height(),
                Some(0),
                "later bodies must not bypass missing block 1"
            );
            assert!(!node
                .query
                .block_queue_has_hash(&blocks[1].block_hash().to_byte_array()));
            if matches!(mode, Replay::SilentHeaders) {
                while stats.header_requests.load(Ordering::SeqCst) < 3 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                tokio::time::sleep(Duration::from_secs(6)).await;
            } else {
                tokio::task::yield_now().await;
            }
            assert_eq!(node.tip_height(), Some(0));
            release
                .send(())
                .map_err(|_| "peer disconnected before withheld frontier release")?;
            while node.tip_height() != Some(6) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(node.hub.tip_hash(), Some(blocks[6].block_hash()));
            if matches!(mode, Replay::InitialPeerEof) {
                assert_eq!(stats.frontier_failures.load(Ordering::SeqCst), 1);
                assert!(
                    stats.frontier_requests.load(Ordering::SeqCst) >= 2,
                    "both the failed and replacement peer must receive getdata for block 1"
                );
            }
            Ok::<(), &str>(())
        })
        .await;
        cancel.store(true, Ordering::SeqCst);
        result
    };
    let result = tokio::time::timeout(Duration::from_secs(40), async {
        tokio::join!(sync, control)
    })
    .await;
    eprintln!("replay={mode:?} header_requests={} frontier_requests={} frontier_failures={} final_tip={:?} server_finished={}", stats.header_requests.load(Ordering::SeqCst), stats.frontier_requests.load(Ordering::SeqCst), stats.frontier_failures.load(Ordering::SeqCst), node.tip_height(), server.is_finished());
    server.abort();
    let _ = server.await;
    node.shutdown().await;
    let (sync_result, control_result) =
        result.expect("replay and cooperative shutdown exceeded deadline");
    control_result
        .expect("later-body queue or full-validation catch-up did not complete")
        .expect("frontier replay failed");
    sync_result.expect("IBD replay failed");
}
