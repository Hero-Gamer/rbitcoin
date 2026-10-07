#![no_main]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use libfuzzer_sys::fuzz_target;
use rbitcoin_consensus::Milestone;
use rbitcoin_fuzz::{
    check_diff_env, cmpct_getblocktxn_agrees, cmpct_missing_for_case, compare_one,
    compare_tx_bytes, diff_regtest_params, encode_cmpctblock_v2, encode_feefilter_payload,
    encode_getheaders_empty_v2, encode_getheaders_v2, encode_inv_payload, encode_ping_v2,
    encode_pong_v2, encode_tx_v2, genesis_diff_tip, header_sequence_agrees, parse_p2p_sequence,
    prepare_cmpct_fuzz_case, spawn_bitcoind_p2p, tmp_dir, BlockOracle, CompareOne, CoreChild,
    DiffTip, P2pSeqKind,
};
use rbitcoin_net::{
    classify_v2_cmpct_peer, v2_header_hashes, ChainHub, CmpctPeerFrame, NetError, P2PNode,
    V2PlainSession,
};
use rbitcoin_query::Query;
use tokio::net::TcpStream;
use tokio::runtime::{Builder, Runtime};

struct Base {
    hub: ChainHub,
    core: CoreChild,
    tip: std::sync::Mutex<DiffTip>,
    rt: Runtime,
    session: Mutex<Option<V2PlainSession>>,
    node: Mutex<P2PNode>,
    _store: PathBuf,
}

static BASE: OnceLock<Base> = OnceLock::new();
static COMPARISONS: AtomicU64 = AtomicU64::new(0);
static PING_SEQ: AtomicU64 = AtomicU64::new(1);
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(10);
const READ_WAIT: Duration = Duration::from_millis(200);

fn harness_failure(what: &str) -> ! {
    eprintln!("=== P2P-SEQUENCE FUZZ HARNESS FAILURE ===");
    eprintln!("{what}");
    std::process::exit(2);
}

fn note_comparison() {
    let n = COMPARISONS.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(100) {
        eprintln!("p2p-sequence: comparisons={n}");
    }
}

fn base() -> &'static Base {
    BASE.get_or_init(|| {
        if std::env::var_os("RBITCOIN_IO").is_none() {
            std::env::set_var("RBITCOIN_IO", "fd");
        }
        let io = std::env::var("RBITCOIN_IO").ok();
        if let Err(e) = check_diff_env(None, io.as_deref()) {
            harness_failure(e);
        }
        let bin = std::env::var("RBITCOIN_CORE_BITCOIND").unwrap_or_default();
        if bin.is_empty() {
            harness_failure("RBITCOIN_CORE_BITCOIND unset");
        }
        let store = tmp_dir("rbtc-p2p-seq-store");
        let core_dir = tmp_dir("rbtc-p2p-seq-core");
        let q = Query::open_or_create_tiny(store.join("store")).unwrap_or_else(|e| {
            harness_failure(&format!("query open: {e}"));
        });
        let params = diff_regtest_params();
        let hub = ChainHub::new(q, params.clone(), Milestone::NONE);
        hub.ensure_genesis()
            .unwrap_or_else(|e| harness_failure(&format!("genesis: {e}")));
        let (core, p2p) = spawn_bitcoind_p2p(std::path::Path::new(&bin), &core_dir)
            .unwrap_or_else(|e| harness_failure(&e));
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| harness_failure(&format!("tokio runtime: {e}")));
        let session = rt
            .block_on(connect_session(p2p))
            .unwrap_or_else(|e| harness_failure(&format!("initial handshake: {e}")));
        let listen_q = Query::open_or_create_tiny(store.join("listen")).unwrap_or_else(|e| {
            harness_failure(&format!("listen query: {e}"));
        });
        let node = rt
            .block_on(P2PNode::start(
                "127.0.0.1:0".parse().unwrap(),
                listen_q,
                params.clone(),
                Milestone::NONE,
            ))
            .unwrap_or_else(|e| harness_failure(&format!("p2p listen: {e}")));
        Base {
            hub,
            core,
            tip: std::sync::Mutex::new(genesis_diff_tip(&params)),
            rt,
            session: Mutex::new(Some(session)),
            node: Mutex::new(node),
            _store: store,
        }
    })
}

async fn connect_session(p2p: SocketAddr) -> Result<V2PlainSession, String> {
    let stream = TcpStream::connect(p2p).await.map_err(|e| e.to_string())?;
    V2PlainSession::outbound_regtest(stream, "/rbitcoin:fuzz/", HANDSHAKE_LIMIT)
        .await
        .map_err(|e| format!("handshake: {e}"))
}

async fn ping_session(sess: &mut V2PlainSession) -> bool {
    let nonce = PING_SEQ.fetch_add(1, Ordering::Relaxed);
    let Ok(ping) = encode_ping_v2(nonce) else {
        return false;
    };
    if sess.write_contents(&ping).await.is_err() {
        return false;
    }
    let deadline = tokio::time::Instant::now() + READ_WAIT;
    while tokio::time::Instant::now() < deadline {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, sess.read_contents()).await {
            Err(_) | Ok(Err(_)) => return false,
            Ok(Ok(contents)) => match classify_v2_cmpct_peer(&contents) {
                CmpctPeerFrame::Pong(n) if n == nonce => return true,
                CmpctPeerFrame::Ping(n) => {
                    let Ok(pong) = encode_pong_v2(n) else {
                        return false;
                    };
                    if sess.write_contents(&pong).await.is_err() {
                        return false;
                    }
                }
                _ => {}
            },
        }
    }
    false
}

fn ping_compare(b: &Base) -> bool {
    let mut slot = b.session.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sess) = slot.as_mut() else {
        return false;
    };
    b.rt.block_on(ping_session(sess))
}

fn headers_live(b: &Base) -> bool {
    let mut slot = b.session.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sess) = slot.as_mut() else {
        return false;
    };
    let Ok(frame) = encode_getheaders_empty_v2() else {
        return false;
    };
    b.rt.block_on(async { sess.write_contents(&frame).await.ok() })
        .is_some()
}

enum StepFate {
    Compared,
    Skip,
    Disagree(String),
}

fn known_headers(b: &Base) -> Vec<BlockHash> {
    let genesis = genesis_diff_tip(&diff_regtest_params()).hash;
    let tip = b.hub.tip_hash().unwrap_or(genesis);
    if tip == genesis {
        vec![genesis]
    } else {
        vec![genesis, tip]
    }
}

fn locator_from_tip(b: &Base, known: &[BlockHash]) -> Vec<BlockHash> {
    let genesis = known[0];
    let mut loc = Vec::new();
    if let Some(tip) = b.hub.tip_hash() {
        loc.push(tip);
    }
    if loc.last() != Some(&genesis) {
        loc.push(genesis);
    }
    if loc.is_empty() {
        loc.push(genesis);
    }
    loc
}

async fn answer_ping(sess: &mut V2PlainSession, contents: &[u8]) -> bool {
    if let CmpctPeerFrame::Ping(n) = classify_v2_cmpct_peer(contents) {
        let Ok(pong) = encode_pong_v2(n) else {
            return false;
        };
        return sess.write_contents(&pong).await.is_ok();
    }
    true
}

fn getheaders_step(b: &Base) -> StepFate {
    let known = known_headers(b);
    let Ok(frame) = encode_getheaders_v2(
        locator_from_tip(b, &known),
        BlockHash::from_byte_array([0; 32]),
    ) else {
        return StepFate::Skip;
    };
    let mut slot = b.session.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sess) = slot.as_mut() else {
        return StepFate::Skip;
    };
    let agreed = b.rt.block_on(async {
        if sess.write_contents(&frame).await.is_err() {
            return None;
        }
        let deadline = tokio::time::Instant::now() + READ_WAIT;
        while tokio::time::Instant::now() < deadline {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, sess.read_contents()).await {
                Err(_) | Ok(Err(_)) => return None,
                Ok(Ok(contents)) => {
                    if let Some(hashes) = v2_header_hashes(&contents) {
                        return Some(header_sequence_agrees(&hashes, &known));
                    }
                    if !answer_ping(sess, &contents).await {
                        return None;
                    }
                }
            }
        }
        None
    });
    match agreed {
        Some(true) => StepFate::Compared,
        Some(false) => StepFate::Disagree("header sequence left the hub".into()),
        None => StepFate::Skip,
    }
}

fn compact_step(b: &Base, payload: &[u8]) -> StepFate {
    let Some(case) = prepare_cmpct_fuzz_case(payload) else {
        return StepFate::Skip;
    };
    let Some(ours) = cmpct_missing_for_case(&case) else {
        return StepFate::Skip;
    };
    let Ok(frame) = encode_cmpctblock_v2(&case.hsi) else {
        return StepFate::Skip;
    };
    let mut slot = b.session.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sess) = slot.as_mut() else {
        return StepFate::Skip;
    };
    let result = b.rt.block_on(async {
        for tx in &case.fill_txs {
            let encoded = encode_tx_v2(tx).map_err(|_| NetError::Protocol("tx encode"))?;
            sess.write_contents(&encoded).await?;
        }
        sess.write_contents(&frame).await?;
        let deadline = tokio::time::Instant::now() + READ_WAIT;
        while tokio::time::Instant::now() < deadline {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, sess.read_contents()).await {
                Err(_) => return Ok(None),
                Ok(Err(e)) => return Err(e),
                Ok(Ok(contents)) => match classify_v2_cmpct_peer(&contents) {
                    CmpctPeerFrame::GetBlockTxn(idx) => return Ok(Some(idx)),
                    CmpctPeerFrame::Ping(n) => {
                        let pong = encode_pong_v2(n).map_err(|_| NetError::Protocol("pong"))?;
                        sess.write_contents(&pong).await?;
                    }
                    CmpctPeerFrame::Pong(_) | CmpctPeerFrame::Other => {}
                },
            }
        }
        Ok(None)
    });
    match result {
        Ok(Some(core_idx)) => {
            if !cmpct_getblocktxn_agrees(&ours, &core_idx) {
                return StepFate::Disagree(format!("ours={ours:?} core={core_idx:?}"));
            }
            let hash = case.hsi.header.block_hash().to_string();
            let _ = b.core.rpc.core_invalidate_hash(&hash);
            StepFate::Compared
        }
        Ok(None) if ours.is_empty() => {
            let hash = case.hsi.header.block_hash().to_string();
            let _ = b.core.rpc.core_invalidate_hash(&hash);
            StepFate::Compared
        }
        _ => StepFate::Skip,
    }
}

fn local_send_and_ping(b: &Base, frame: &[u8]) -> bool {
    let addr = b.node.lock().unwrap_or_else(|e| e.into_inner()).local_addr;
    b.rt.block_on(async {
        let Ok(stream) = TcpStream::connect(addr).await else {
            return false;
        };
        let Ok(mut sess) =
            V2PlainSession::outbound_regtest(stream, "/rbitcoin:fuzz/", HANDSHAKE_LIMIT).await
        else {
            return false;
        };
        if sess.write_contents(frame).await.is_err() {
            return false;
        }
        ping_session(&mut sess).await
    })
}

fn core_send_and_ping(b: &Base, frame: &[u8]) -> Option<bool> {
    let mut slot = b.session.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sess) = slot.as_mut() else {
        return None;
    };
    let up = b.rt.block_on(async {
        if sess.write_contents(frame).await.is_err() {
            return false;
        }
        ping_session(sess).await
    });
    if !up {
        if let Some(mut dead) = slot.take() {
            dead.close();
        }
    }
    Some(up)
}

fn relay_step(b: &Base, payload: &[u8], fee: bool) -> StepFate {
    let Some(frame) = (if fee {
        encode_feefilter_payload(payload)
    } else {
        encode_inv_payload(payload)
    }) else {
        return StepFate::Skip;
    };
    let local_up = local_send_and_ping(b, &frame);
    let Some(core_up) = core_send_and_ping(b, &frame) else {
        return StepFate::Skip;
    };
    if local_up == core_up {
        StepFate::Compared
    } else {
        StepFate::Disagree(format!("local_up={local_up} core_up={core_up}"))
    }
}

fn finish_step(fate: StepFate, what: &str) {
    match fate {
        StepFate::Compared => note_comparison(),
        StepFate::Skip => {}
        StepFate::Disagree(detail) => panic!("p2p-sequence {what}: {detail}"),
    }
}

fuzz_target!(|data: &[u8]| {
    let b = base();
    let steps = parse_p2p_sequence(data);
    let mut tip = b.tip.lock().unwrap_or_else(|e| e.into_inner());
    for step in steps {
        if step.skip {
            continue;
        }
        match step.kind {
            P2pSeqKind::Ping => {
                if ping_compare(b) {
                    note_comparison();
                }
            }
            P2pSeqKind::Headers => {
                let _ = headers_live(b);
            }
            P2pSeqKind::Block => {
                let body = if data.first() == Some(&rbitcoin_fuzz::P2P_SEQ_IR) {
                    step.payload.as_slice()
                } else {
                    data
                };
                match compare_one(&b.hub, &mut tip, &b.core.rpc, body) {
                    CompareOne::Agreed { .. } => note_comparison(),
                    CompareOne::NotABlock | CompareOne::Skipped => {}
                    CompareOne::Disagreed { ours, core, hex } => {
                        panic!("p2p-sequence block: ours={ours} core={core} hex={hex}");
                    }
                    CompareOne::Harness(msg) if msg == "oracle dead" => {}
                    CompareOne::Harness(msg) => harness_failure(msg),
                }
            }
            P2pSeqKind::Tx => match compare_tx_bytes(&b.hub, &b.core.rpc, &step.payload) {
                CompareOne::Agreed { .. } => note_comparison(),
                CompareOne::NotABlock | CompareOne::Skipped => {}
                CompareOne::Disagreed { ours, core, hex } => {
                    panic!("p2p-sequence tx: ours={ours} core={core} hex={hex}");
                }
                CompareOne::Harness(msg) if msg == "oracle dead" => {}
                CompareOne::Harness(msg) => harness_failure(msg),
            },
            P2pSeqKind::GetHeaders => finish_step(getheaders_step(b), "getheaders"),
            P2pSeqKind::Cmpct | P2pSeqKind::Blocktxn => {
                finish_step(compact_step(b, &step.payload), "cmpct");
            }
            P2pSeqKind::FeeFilter => finish_step(relay_step(b, &step.payload, true), "feefilter"),
            P2pSeqKind::Inv => finish_step(relay_step(b, &step.payload, false), "inv"),
        }
    }
});
