//! A responsive supplier must not be starved by a peer withholding only block 1.
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

#[derive(Clone, Copy)]
enum Role {
    Withholder,
    Supplier,
}

#[derive(Default)]
struct Observed {
    headers: AtomicUsize,
    frontier: AtomicUsize,
    bodies: AtomicUsize,
    connected: AtomicBool,
    closed: AtomicBool,
}

async fn serve(
    listener: TcpListener,
    blocks: Arc<Vec<Block>>,
    role: Role,
    observed: Arc<Observed>,
    mut ready: oneshot::Receiver<()>,
) -> Result<(), rbitcoin_net::error::NetError> {
    if matches!(role, Role::Supplier) {
        // Reject the initial dial, so block 1 is first assigned to the withholder.
        loop {
            tokio::select! {
                result = &mut ready => {
                    if result.is_err() { return Ok(()); }
                    break;
                }
                result = listener.accept() => {
                    let (early, _) = result?;
                    drop(early);
                }
            }
        }
    }
    let (stream, remote) = listener.accept().await?;
    let local = stream.local_addr()?;
    let (_, mut reader, mut writer, _, _shutdown) = connect_and_handshake_timed(
        Duration::from_secs(5),
        stream,
        Magic::REGTEST,
        local,
        remote,
        (blocks.len() + 64) as i32,
        true,
        "/competing-frontier/",
        HandshakePolicy::plain(),
    )
    .await?;
    observed.connected.store(true, Ordering::SeqCst);
    let mut pending: VecDeque<usize> = VecDeque::new();
    let mut sends = tokio::time::interval(Duration::from_millis(5));
    loop {
        let frame = tokio::select! {
            _ = sends.tick(), if !pending.is_empty() => {
                let height = pending.pop_front().unwrap();
                write_v2_msg(&mut writer, NetworkMessage::Block(blocks[height].clone())).await?;
                observed.bodies.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            result = read_v2_frame(&mut reader, Magic::REGTEST) => result?,
        };
        let message = rbitcoin_net::msg_decode::decode_framed_offload(frame).await?;
        match message.payload() {
            NetworkMessage::GetHeaders(request) => {
                let previous = observed.headers.fetch_add(1, Ordering::SeqCst);
                if previous > 0 && matches!(role, Role::Withholder) {
                    continue;
                }
                let start = blocks
                    .iter()
                    .rposition(|block| request.locator_hashes.contains(&block.block_hash()))
                    .map_or(1, |height| height + 1);
                write_v2_msg(
                    &mut writer,
                    NetworkMessage::Headers(
                        blocks[start..].iter().map(|block| block.header).collect(),
                    ),
                )
                .await?;
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
                        observed.frontier.fetch_add(1, Ordering::SeqCst);
                        if matches!(role, Role::Withholder) {
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

fn spawn_server(
    listener: TcpListener,
    blocks: Arc<Vec<Block>>,
    role: Role,
    observed: Arc<Observed>,
    ready: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<Result<(), rbitcoin_net::error::NetError>> {
    tokio::spawn(async move {
        let result = serve(listener, blocks, role, observed.clone(), ready).await;
        observed.closed.store(true, Ordering::SeqCst);
        result
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in delayed-supplier BIP324 replay; default coverage backdates header deadlines and exercises assignment directly"]
async fn healthy_supplier_gets_frontier_below_racing_threshold() {
    competing(32).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in 1100-block BIP324 replay; default coverage queues 1000 tiny bodies without mining or wall-clock sleeps"]
async fn healthy_supplier_gets_frontier_above_racing_threshold() {
    competing(1100).await;
}

async fn competing(count: u32) {
    let dir = rbitcoin_store::testutil::TempDir::labeled("competing-frontier").unwrap();
    let node = P2PNode::start(
        "127.0.0.1:0".parse().unwrap(),
        Query::open_or_create_tiny(dir.join("store")).unwrap(),
        ChainParams::regtest(),
        Milestone::NONE,
        rbitcoin_net::REDIAL_INTERVAL_TEST,
    )
    .await
    .unwrap();
    let blocks = Arc::new(super::frontier_replay::chain(count));
    let primary = Arc::new(Observed::default());
    let supplier = Arc::new(Observed::default());
    let primary_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let supplier_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addresses = [
        primary_listener.local_addr().unwrap(),
        supplier_listener.local_addr().unwrap(),
    ]
    .map(rbitcoin_net::NetAddr::Ip);
    let (_hold, unused) = oneshot::channel();
    let (ready, ready_rx) = oneshot::channel();
    let primary_server = spawn_server(
        primary_listener,
        blocks.clone(),
        Role::Withholder,
        primary.clone(),
        unused,
    );
    let supplier_server = spawn_server(
        supplier_listener,
        blocks.clone(),
        Role::Supplier,
        supplier.clone(),
        ready_rx,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let mut cfg = IbdConfig::for_test();
    cfg.target_peers = 2;
    cfg.window = 256;
    cfg.per_peer = 64;
    cfg.stall = Duration::from_secs(60);
    cfg.connect_timeout = Duration::from_secs(5);
    cfg.peerless = rbitcoin_ibd::Peerless::Redial {
        cap: Duration::from_millis(100),
    };
    let sync = rbitcoin_ibd::sync_cancellable(&node, &addresses, cfg, Some(cancel.clone()));
    let control = async {
        let result = tokio::time::timeout(Duration::from_secs(40), async {
            while primary.frontier.load(Ordering::SeqCst) == 0 || primary.bodies.load(Ordering::SeqCst) < 16 || primary.headers.load(Ordering::SeqCst) < 3 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_secs(6)).await;
            let queued = blocks[2..].iter().filter(|block| node.query.block_queue_has_hash(&block.block_hash().to_byte_array())).count();
            eprintln!("competition count={count} before_supplier queued={queued} tip={:?} withholder_closed={}", node.tip_height(), primary.closed.load(Ordering::SeqCst));
            if node.tip_height() != Some(0) { return Err("fixture advanced before supplier was available"); }
            if count > 1000 && queued < 1000 { return Err("high-queue fixture did not cross the racing threshold"); }
            ready.send(()).map_err(|_| "supplier readiness receiver closed")?;
            while !supplier.connected.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
            let requested_at = std::time::Instant::now();
            let frontier_result = tokio::time::timeout(Duration::from_secs(8), async {
                while supplier.frontier.load(Ordering::SeqCst) == 0 { tokio::time::sleep(Duration::from_millis(10)).await; }
            }).await;
            eprintln!("competition count={count} handoff_wait_ms={} supplier_frontier={} withholder_closed_before_cancel={}", requested_at.elapsed().as_millis(), supplier.frontier.load(Ordering::SeqCst), primary.closed.load(Ordering::SeqCst));
            frontier_result.map_err(|_| "healthy connected supplier was not asked for block 1 within 8s")?;
            if primary.closed.load(Ordering::SeqCst) { return Err("handoff must not rely on dropping the productive connection"); }
            while node.tip_height() != Some(count) { tokio::time::sleep(Duration::from_millis(10)).await; }
            if node.hub.tip_hash() != Some(blocks[count as usize].block_hash()) { return Err("unexpected final tip hash"); }
            Ok::<(), &str>(())
        }).await;
        cancel.store(true, Ordering::SeqCst);
        result
    };
    let result = tokio::time::timeout(Duration::from_secs(45), async {
        tokio::join!(sync, control)
    })
    .await;
    eprintln!("competition count={count} final_tip={:?} supplier_connected={} supplier_frontier={} primary_frontier={} primary_bodies={} primary_closed={}", node.tip_height(), supplier.connected.load(Ordering::SeqCst), supplier.frontier.load(Ordering::SeqCst), primary.frontier.load(Ordering::SeqCst), primary.bodies.load(Ordering::SeqCst), primary.closed.load(Ordering::SeqCst));
    primary_server.abort();
    supplier_server.abort();
    let _ = primary_server.await;
    let _ = supplier_server.await;
    node.shutdown().await;
    let (sync_result, control_result) = result.expect("competition teardown deadline exceeded");
    control_result
        .expect("competition setup or validation deadline exceeded")
        .expect("healthy supplier was starved");
    sync_result.expect("IBD failed during supplier competition");
}
