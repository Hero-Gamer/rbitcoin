//! Bitcoin P2P message framing helpers (limits + [`FramedMessage`]).
//!
//! **Transport:** production wire is **BIP324 v2 only** — see [`crate::v2`].
//! This module holds Core-aligned size limits, CPU-heavy encode/decode hints,
//! and the framed-message type used after decrypt.
//!
//! **I/O vs CPU split:** socket tasks obtain a [`FramedMessage`] via
//! [`crate::v2::read_v2_frame`] and run [`FramedMessage::decode`] on a
//! blocking worker. Never deserialize multi‑MB `block` payloads on the async
//! I/O worker.

use bitcoin::consensus::encode::Decodable;
use bitcoin::p2p::message::{CommandString, NetworkMessage, RawNetworkMessage};
use bitcoin::p2p::Magic;

/// Bitcoin Core `MAX_PROTOCOL_MESSAGE_LENGTH` — max P2P payload bytes.
///
/// Note: rust-bitcoin's `MAX_MSG_SIZE` is 5_000_000; Core enforces 4_000_000.
/// We use Core's limit so we never accept (or send) messages Core would reject.
pub const MAX_PROTOCOL_MESSAGE_LENGTH: usize = 4_000_000;

/// Bitcoin Core `MAX_INV_SZ` — max inventory items in inv/getdata/notfound.
pub const MAX_INV_SIZE: usize = 50_000;

/// Tx announcements per `inv`. Separate from [`MAX_INV_SIZE`]: one message
/// stays at a thousand even though the wire cap is fifty thousand.
pub const TX_INV_BATCH: usize = 1_000;

/// Bitcoin Core `MAX_HEADERS_RESULTS` — max headers in a `headers` message.
pub const MAX_HEADERS_RESULTS: usize = 2_000;

/// Bitcoin Core `MAX_LOCATOR_SZ` — max block locator hashes.
pub const MAX_LOCATOR_SZ: usize = 101;

/// Whether encoding this payload is heavy enough to keep off async I/O workers.
#[inline]
pub fn encode_is_cpu_heavy(payload: &NetworkMessage) -> bool {
    match payload {
        NetworkMessage::Block(_) | NetworkMessage::Headers(_) => true,
        NetworkMessage::Inv(v) | NetworkMessage::GetData(v) | NetworkMessage::NotFound(v) => {
            v.len() > 64
        }
        _ => false,
    }
}

/// One fully framed P2P message: header fields + raw payload bytes.
///
/// Produced on the socket task; [`FramedMessage::decode`] is CPU and must run
/// off the async I/O worker (blocking pool / rayon / dedicated thread).
#[derive(Debug, Clone)]
pub struct FramedMessage {
    pub magic: Magic,
    /// 12-byte null-padded command (wire form).
    pub command: [u8; 12],
    pub payload: Vec<u8>,
}

impl FramedMessage {
    #[inline]
    pub fn is_block(&self) -> bool {
        self.command == *b"block\0\0\0\0\0\0\0"
    }

    #[inline]
    pub fn is_headers(&self) -> bool {
        self.command == *b"headers\0\0\0\0\0"
    }

    #[inline]
    /// Framed application payload length (for per-peer byte rate accounting).
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    pub fn is_ping(&self) -> bool {
        self.command == *b"ping\0\0\0\0\0\0\0\0"
    }

    pub fn is_pong(&self) -> bool {
        self.command == *b"pong\0\0\0\0\0\0\0\0"
    }

    #[inline]
    pub fn is_notfound(&self) -> bool {
        self.command == *b"notfound\0\0\0\0"
    }

    #[inline]
    fn is_inv_like(&self) -> bool {
        self.command == *b"inv\0\0\0\0\0\0\0\0\0"
            || self.command == *b"getdata\0\0\0\0\0"
            || self.is_notfound()
    }

    /// Cheap ping nonce extract (8-byte LE payload). No full message deserialize.
    pub fn ping_nonce(&self) -> Option<u64> {
        if !self.is_ping() || self.payload.len() < 8 {
            return None;
        }
        Some(u64::from_le_bytes(self.payload[..8].try_into().ok()?))
    }

    /// Block hash from the wire header (first 80 payload bytes) — no full deserialize.
    ///
    /// Used so IBD can free getdata in-flight as soon as the TCP frame is complete,
    /// while `block` payload decode still runs on the blocking pool. Waiting for
    /// full deserialize to free slots made healthy peers look stalled (socket idle
    /// with `in_flight` still full).
    pub fn block_hash_from_header(&self) -> Option<bitcoin::BlockHash> {
        if !self.is_block() || self.payload.len() < 80 {
            return None;
        }
        use bitcoin::hashes::{sha256d, Hash as _};
        let header = &self.payload[..80];
        let dig = sha256d::Hash::hash(header);
        Some(bitcoin::BlockHash::from_byte_array(dig.to_byte_array()))
    }

    /// Extra bytes / unknown command → [`NetworkMessage::Unknown`].
    /// Oversize `headers` (`n > MAX_HEADERS_RESULTS`), oversize inv-like
    /// messages, and a `tx` / `block` / `cmpctblock` / `blocktxn` whose
    /// compact-size count cannot fit in the remaining bytes are
    /// [`NetError::MessageTooLarge`] before `consensus_decode` allocates.
    pub fn try_decode(self) -> Result<RawNetworkMessage, crate::error::NetError> {
        if self.is_headers() {
            let mut sl = self.payload.as_slice();
            if let Ok(n) = bitcoin::consensus::encode::VarInt::consensus_decode(&mut sl) {
                let n = n.0 as usize;
                if n > MAX_HEADERS_RESULTS {
                    return Err(crate::error::NetError::MessageTooLarge(n));
                }
            }
        }
        if self.is_inv_like() {
            let mut sl = self.payload.as_slice();
            if let Ok(n) = bitcoin::consensus::encode::VarInt::consensus_decode(&mut sl) {
                let n = n.0 as usize;
                if n > MAX_INV_SIZE {
                    return Err(crate::error::NetError::MessageTooLarge(n));
                }
            }
        }
        if let Some(n) = tx_family_too_large(&self.command, &self.payload) {
            return Err(crate::error::NetError::MessageTooLarge(n));
        }
        Ok(self.decode())
    }

    /// Deserialize the application payload (no v1 header, no checksum).
    ///
    /// Extra bytes / unknown command → [`NetworkMessage::Unknown`].
    pub fn decode(self) -> RawNetworkMessage {
        let cmd = command_from_header(&self.command);
        let mut sl = self.payload.as_slice();
        match decode_cmd_payload(cmd.as_ref(), &mut sl) {
            Ok(Some(msg)) if sl.is_empty() => RawNetworkMessage::new(self.magic, msg),
            _ => RawNetworkMessage::new(
                self.magic,
                NetworkMessage::Unknown {
                    command: cmd,
                    payload: self.payload,
                },
            ),
        }
    }

    /// Commands whose decode cost should never run on an async I/O worker.
    #[inline]
    pub fn decode_is_cpu_heavy(&self) -> bool {
        self.is_block() || self.is_headers() || self.is_notfound()
    }
}

/// Core `CMessageHeader::IsCommandValid`: printable ASCII (0x20–0x7E), null-padded.
///
/// Digits are required — long-form commands include `sendaddrv2` (and short-id
/// names like `addrv2` when encoded long). Restricting to a–z rejected those
/// and killed post-handshake IBD peers that send `sendaddrv2`.
pub(crate) fn command_bytes_ok(cmd12: &[u8]) -> bool {
    if cmd12.len() != 12 {
        return false;
    }
    let mut seen_null = false;
    let mut any = false;
    for &b in cmd12 {
        if b == 0 {
            seen_null = true;
            continue;
        }
        if seen_null {
            return false;
        }
        if !b.is_ascii_graphic() && b != b' ' {
            return false;
        }
        any = true;
    }
    any
}

/// Outpoint, empty script compact-size, and sequence.
const MIN_INPUT_BYTES: u64 = 41;
/// Value plus an empty script compact-size.
const MIN_OUTPUT_BYTES: u64 = 9;
/// Marker, flag, empty input list, empty output list, and locktime.
const MIN_TX_BYTES: u64 = 12;

enum Bound {
    Fits,
    /// Compact-size count cannot fit in the bytes still unread.
    TooLarge(usize),
    /// Truncated or non-minimal. `consensus_decode` decides.
    Undecided,
}

fn too_large_n(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

fn exceeds(count: u64, min_each: u64, remaining: usize) -> bool {
    count.saturating_mul(min_each) > u64::try_from(remaining).unwrap_or(u64::MAX)
}

fn skip(data: &mut &[u8], n: usize) -> bool {
    if data.len() < n {
        return false;
    }
    *data = &data[n..];
    true
}

fn read_compact_size(data: &mut &[u8]) -> Option<u64> {
    let mut cur = *data;
    match bitcoin::consensus::encode::VarInt::consensus_decode(&mut cur) {
        Ok(n) => {
            *data = cur;
            Some(n.0)
        }
        Err(_) => None,
    }
}

fn take_len_bytes(data: &mut &[u8]) -> Bound {
    let Some(n) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    if n > u64::try_from(data.len()).unwrap_or(u64::MAX) {
        return Bound::TooLarge(too_large_n(n));
    }
    let Some(n) = usize::try_from(n).ok() else {
        return Bound::TooLarge(usize::MAX);
    };
    if skip(data, n) {
        Bound::Fits
    } else {
        Bound::Undecided
    }
}

fn walk_input(data: &mut &[u8]) -> Bound {
    if !skip(data, 36) {
        return Bound::Undecided;
    }
    match take_len_bytes(data) {
        Bound::Fits => {}
        other => return other,
    }
    if skip(data, 4) {
        Bound::Fits
    } else {
        Bound::Undecided
    }
}

fn walk_output(data: &mut &[u8]) -> Bound {
    if !skip(data, 8) {
        return Bound::Undecided;
    }
    take_len_bytes(data)
}

fn walk_counted(
    data: &mut &[u8],
    count: u64,
    min_each: u64,
    one: fn(&mut &[u8]) -> Bound,
) -> Bound {
    if exceeds(count, min_each, data.len()) {
        return Bound::TooLarge(too_large_n(count));
    }
    let Some(n) = usize::try_from(count).ok() else {
        return Bound::TooLarge(usize::MAX);
    };
    for _ in 0..n {
        match one(data) {
            Bound::Fits => {}
            other => return other,
        }
    }
    Bound::Fits
}

/// Witness stack. Each element is at least one byte, so a count past the
/// remaining bytes is rejected before `Witness::consensus_decode` allocates
/// `count * 4 + 128`.
fn walk_witness(data: &mut &[u8]) -> Bound {
    let Some(count) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    if count > u64::try_from(data.len()).unwrap_or(u64::MAX) {
        return Bound::TooLarge(too_large_n(count));
    }
    let Some(n) = usize::try_from(count).ok() else {
        return Bound::TooLarge(usize::MAX);
    };
    for _ in 0..n {
        match take_len_bytes(data) {
            Bound::Fits => {}
            other => return other,
        }
    }
    Bound::Fits
}

fn walk_tx(data: &mut &[u8]) -> Bound {
    if !skip(data, 4) {
        return Bound::Undecided;
    }
    let Some(marker_or_vin) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    let (vin, segwit) = if marker_or_vin == 0 {
        let Some((flag, rest)) = data.split_first() else {
            return Bound::Undecided;
        };
        *data = rest;
        if *flag != 1 {
            return Bound::Undecided;
        }
        let Some(vin) = read_compact_size(data) else {
            return Bound::Undecided;
        };
        (vin, true)
    } else {
        (marker_or_vin, false)
    };
    match walk_counted(data, vin, MIN_INPUT_BYTES, walk_input) {
        Bound::Fits => {}
        other => return other,
    }
    let Some(vout) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    match walk_counted(data, vout, MIN_OUTPUT_BYTES, walk_output) {
        Bound::Fits => {}
        other => return other,
    }
    if segwit {
        let Some(n) = usize::try_from(vin).ok() else {
            return Bound::TooLarge(usize::MAX);
        };
        for _ in 0..n {
            match walk_witness(data) {
                Bound::Fits => {}
                other => return other,
            }
        }
    }
    if skip(data, 4) {
        Bound::Fits
    } else {
        Bound::Undecided
    }
}

fn walk_tx_list(data: &mut &[u8], count: u64) -> Bound {
    if exceeds(count, MIN_TX_BYTES, data.len()) {
        return Bound::TooLarge(too_large_n(count));
    }
    let Some(n) = usize::try_from(count).ok() else {
        return Bound::TooLarge(usize::MAX);
    };
    for _ in 0..n {
        match walk_tx(data) {
            Bound::Fits => {}
            other => return other,
        }
    }
    Bound::Fits
}

fn walk_cmpctblock(data: &mut &[u8]) -> Bound {
    if !skip(data, 88) {
        return Bound::Undecided;
    }
    let Some(shorts) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    if exceeds(shorts, 6, data.len()) {
        return Bound::TooLarge(too_large_n(shorts));
    }
    let Some(nbytes) = usize::try_from(shorts).ok().and_then(|n| n.checked_mul(6)) else {
        return Bound::TooLarge(usize::MAX);
    };
    if !skip(data, nbytes) {
        return Bound::Undecided;
    }
    let Some(prefills) = read_compact_size(data) else {
        return Bound::Undecided;
    };
    if exceeds(prefills, 1 + MIN_TX_BYTES, data.len()) {
        return Bound::TooLarge(too_large_n(prefills));
    }
    let Some(n) = usize::try_from(prefills).ok() else {
        return Bound::TooLarge(usize::MAX);
    };
    for _ in 0..n {
        if read_compact_size(data).is_none() {
            return Bound::Undecided;
        }
        match walk_tx(data) {
            Bound::Fits => {}
            other => return other,
        }
    }
    Bound::Fits
}

/// `Some(count)` when a transaction-carrying payload declares a compact-size
/// count the remaining bytes cannot hold. `None` lets `decode` run.
fn tx_family_too_large(command: &[u8; 12], payload: &[u8]) -> Option<usize> {
    let mut data = payload;
    let bound = if command == b"tx\0\0\0\0\0\0\0\0\0\0" {
        walk_tx(&mut data)
    } else if command == b"blocktxn\0\0\0\0" {
        if !skip(&mut data, 32) {
            return None;
        }
        let n = read_compact_size(&mut data)?;
        walk_tx_list(&mut data, n)
    } else if command == b"block\0\0\0\0\0\0\0" {
        if !skip(&mut data, 80) {
            return None;
        }
        let n = read_compact_size(&mut data)?;
        walk_tx_list(&mut data, n)
    } else if command == b"cmpctblock\0\0" {
        walk_cmpctblock(&mut data)
    } else {
        return None;
    };
    match bound {
        Bound::TooLarge(n) => Some(n),
        Bound::Fits | Bound::Undecided => None,
    }
}

fn decode_cmd_payload(
    cmd: &str,
    d: &mut &[u8],
) -> Result<Option<NetworkMessage>, bitcoin::consensus::encode::Error> {
    fn one<T: Decodable>(
        d: &mut &[u8],
        f: fn(T) -> NetworkMessage,
    ) -> Result<NetworkMessage, bitcoin::consensus::encode::Error> {
        Ok(f(Decodable::consensus_decode(d)?))
    }
    Ok(Some(match cmd {
        "verack" => NetworkMessage::Verack,
        "sendheaders" => NetworkMessage::SendHeaders,
        "getaddr" => NetworkMessage::GetAddr,
        "mempool" => NetworkMessage::MemPool,
        "filterclear" => NetworkMessage::FilterClear,
        "wtxidrelay" => NetworkMessage::WtxidRelay,
        "sendaddrv2" => NetworkMessage::SendAddrV2,
        "version" => one(d, NetworkMessage::Version)?,
        "addr" => one(d, NetworkMessage::Addr)?,
        "inv" => one(d, NetworkMessage::Inv)?,
        "getdata" => one(d, NetworkMessage::GetData)?,
        "notfound" => one(d, NetworkMessage::NotFound)?,
        "getblocks" => one(d, NetworkMessage::GetBlocks)?,
        "getheaders" => one(d, NetworkMessage::GetHeaders)?,
        "block" => one(d, NetworkMessage::Block)?,
        "tx" => one(d, NetworkMessage::Tx)?,
        "ping" => one(d, NetworkMessage::Ping)?,
        "pong" => one(d, NetworkMessage::Pong)?,
        // Bloom is off. `PartialMerkleTree` allocates `n * 8` bools from the
        // flags compact-size before a short payload can fail the read.
        "merkleblock" => return Ok(None),
        "filterload" => one(d, NetworkMessage::FilterLoad)?,
        "filteradd" => one(d, NetworkMessage::FilterAdd)?,
        "getcfilters" => one(d, NetworkMessage::GetCFilters)?,
        "cfilter" => one(d, NetworkMessage::CFilter)?,
        "getcfheaders" => one(d, NetworkMessage::GetCFHeaders)?,
        "cfheaders" => one(d, NetworkMessage::CFHeaders)?,
        "getcfcheckpt" => one(d, NetworkMessage::GetCFCheckpt)?,
        "cfcheckpt" => one(d, NetworkMessage::CFCheckpt)?,
        "reject" => one(d, NetworkMessage::Reject)?,
        "alert" => one(d, NetworkMessage::Alert)?,
        "sendcmpct" => one(d, NetworkMessage::SendCmpct)?,
        "cmpctblock" => one(d, NetworkMessage::CmpctBlock)?,
        "getblocktxn" => one(d, NetworkMessage::GetBlockTxn)?,
        "blocktxn" => one(d, NetworkMessage::BlockTxn)?,
        "addrv2" => one(d, NetworkMessage::AddrV2)?,
        "feefilter" => {
            let fee: i64 = Decodable::consensus_decode(d)?;
            let upper: i64 = bitcoin::Amount::MAX_MONEY
                .to_sat()
                .try_into()
                .expect("Amount::MAX_MONEY < i64::MAX");
            if fee < 0 || fee > upper {
                return Err(bitcoin::consensus::encode::Error::ParseFailed(
                    "feefilter value out of range",
                ));
            }
            NetworkMessage::FeeFilter(fee)
        }
        "headers" => {
            let n = bitcoin::consensus::encode::VarInt::consensus_decode(d)?.0 as usize;
            if n > MAX_HEADERS_RESULTS {
                return Err(bitcoin::consensus::encode::Error::ParseFailed(
                    "too many headers",
                ));
            }
            let mut hs = Vec::with_capacity(n);
            for _ in 0..n {
                hs.push(bitcoin::block::Header::consensus_decode(d)?);
                let txn: u8 = Decodable::consensus_decode(d)?;
                if txn != 0 {
                    return Err(bitcoin::consensus::encode::Error::ParseFailed(
                        "Headers message should not contain transactions",
                    ));
                }
            }
            NetworkMessage::Headers(hs)
        }
        _ => return Ok(None),
    }))
}

fn command_from_header(cmd12: &[u8]) -> CommandString {
    let end = cmd12.iter().position(|&b| b == 0).unwrap_or(12);
    let s = std::str::from_utf8(&cmd12[..end]).unwrap_or("unknown");
    CommandString::try_from(s)
        .unwrap_or_else(|_| CommandString::try_from("unknown").expect("literal command string"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::blockdata::constants::genesis_block;
    use bitcoin::consensus::serialize;
    use bitcoin::Network;

    fn signet_magic() -> Magic {
        Magic::from(Network::Signet)
    }

    #[test]
    fn frame_decode_verack_ignores_checksum() {
        let frame = FramedMessage {
            magic: signet_magic(),
            command: *b"verack\0\0\0\0\0\0",
            payload: Vec::new(),
        };
        assert!(!frame.decode_is_cpu_heavy());
        assert!(matches!(frame.decode().payload(), NetworkMessage::Verack));
    }

    #[test]
    fn block_hash_from_header_matches_full_block() {
        let magic = Magic::from(Network::Bitcoin);
        let genesis = genesis_block(Network::Bitcoin);
        let want = genesis.block_hash();
        let payload = serialize(&genesis);
        let frame = FramedMessage {
            magic,
            command: *b"block\0\0\0\0\0\0\0",
            payload,
        };
        assert!(frame.is_block());
        assert_eq!(frame.block_hash_from_header().expect("header hash"), want);
        match frame.decode().payload() {
            NetworkMessage::Block(b) => assert_eq!(b.block_hash(), want),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn command_bytes_ok_accepts_null_padded() {
        assert!(command_bytes_ok(b"version\0\0\0\0\0"));
        assert!(command_bytes_ok(b"ping\0\0\0\0\0\0\0\0"));
        // Digits: BIP155 sendaddrv2 / addrv2 (long form).
        assert!(command_bytes_ok(b"sendaddrv2\0\0"));
        assert!(command_bytes_ok(b"addrv2\0\0\0\0\0\0"));
        assert!(!command_bytes_ok(b"\xff\xfe\0\0\0\0\0\0\0\0\0\0"));
        assert!(!command_bytes_ok(b"ping\0x\0\0\0\0\0\0\0")); // non-zero after null
        assert!(!command_bytes_ok(b"\x01ping\0\0\0\0\0\0\0")); // control char
    }

    #[test]
    fn core_limits_documented() {
        assert_eq!(MAX_PROTOCOL_MESSAGE_LENGTH, 4_000_000);
        assert_eq!(MAX_INV_SIZE, 50_000);
        let mut payload = vec![0xfd];
        payload.extend_from_slice(&50_001u16.to_le_bytes());
        let frame = FramedMessage {
            magic: Magic::from(bitcoin::Network::Regtest),
            command: *b"inv\0\0\0\0\0\0\0\0\0",
            payload: payload.clone(),
        };
        match frame.try_decode() {
            Err(crate::error::NetError::MessageTooLarge(n)) => assert_eq!(n, 50_001),
            other => panic!("oversize inv must fail decode, got {other:?}"),
        }
        let mut at_cap = vec![0xfd];
        at_cap.extend_from_slice(&50_000u16.to_le_bytes());
        let at_cap = FramedMessage {
            magic: Magic::from(bitcoin::Network::Regtest),
            command: *b"inv\0\0\0\0\0\0\0\0\0",
            payload: at_cap,
        };
        assert!(
            !matches!(
                at_cap.try_decode(),
                Err(crate::error::NetError::MessageTooLarge(_))
            ),
            "exactly 50_000 inv entries is still a message"
        );
        for command in [*b"getdata\0\0\0\0\0", *b"notfound\0\0\0\0"] {
            let over = FramedMessage {
                magic: Magic::from(bitcoin::Network::Regtest),
                command,
                payload: payload.clone(),
            };
            match over.try_decode() {
                Err(crate::error::NetError::MessageTooLarge(n)) => assert_eq!(n, 50_001),
                other => panic!("oversize {command:?} must fail decode, got {other:?}"),
            }
        }
        let tx = FramedMessage {
            magic: Magic::from(bitcoin::Network::Regtest),
            command: *b"tx\0\0\0\0\0\0\0\0\0\0",
            payload,
        };
        assert!(
            !matches!(
                tx.try_decode(),
                Err(crate::error::NetError::MessageTooLarge(_))
            ),
            "a tx payload is not an inv count"
        );
        assert_eq!(MAX_HEADERS_RESULTS, 2_000);
        assert_eq!(MAX_LOCATOR_SZ, 101);
        // Stricter than rust-bitcoin's 5MB
        const {
            assert!(MAX_PROTOCOL_MESSAGE_LENGTH < bitcoin::p2p::message::MAX_MSG_SIZE);
        }
    }

    #[test]
    fn headers_count_over_2000_is_message_too_large() {
        use crate::error::NetError;
        use bitcoin::consensus::encode::{Encodable, VarInt};

        fn count_payload(n: u64) -> Vec<u8> {
            let mut v = Vec::new();
            VarInt(n).consensus_encode(&mut v).unwrap();
            v
        }

        let over = FramedMessage {
            magic: signet_magic(),
            command: *b"headers\0\0\0\0\0",
            payload: count_payload((MAX_HEADERS_RESULTS as u64) + 1),
        };
        match over.try_decode() {
            Err(NetError::MessageTooLarge(n)) => assert_eq!(n, MAX_HEADERS_RESULTS + 1),
            other => panic!("expected MessageTooLarge, got {other:?}"),
        }

        let at_cap = FramedMessage {
            magic: signet_magic(),
            command: *b"headers\0\0\0\0\0",
            payload: count_payload(MAX_HEADERS_RESULTS as u64),
        };
        assert!(
            !matches!(at_cap.try_decode(), Err(NetError::MessageTooLarge(_))),
            "n=2000 must not be MessageTooLarge"
        );
    }

    #[test]
    fn frame_helpers_ping_headers_notfound_and_encode_cost() {
        let magic = signet_magic();
        let nonce: u64 = 0x1122_3344_5566_7788;
        let payload = nonce.to_le_bytes().to_vec();
        let ping = FramedMessage {
            magic,
            command: *b"ping\0\0\0\0\0\0\0\0",
            payload: payload.clone(),
        };
        assert!(ping.is_ping());
        assert_eq!(ping.ping_nonce(), Some(nonce));
        assert!(!ping.decode_is_cpu_heavy());

        let short = FramedMessage {
            magic,
            command: *b"ping\0\0\0\0\0\0\0\0",
            payload: vec![1, 2, 3],
        };
        assert!(short.ping_nonce().is_none());

        let headers = FramedMessage {
            magic,
            command: *b"headers\0\0\0\0\0",
            payload: vec![],
        };
        assert!(headers.is_headers());
        assert!(headers.decode_is_cpu_heavy());
        assert!(headers.block_hash_from_header().is_none());

        let nf = FramedMessage {
            magic,
            command: *b"notfound\0\0\0\0",
            payload: vec![],
        };
        assert!(nf.is_notfound());
        assert!(nf.decode_is_cpu_heavy());

        // Corrupt payload → Unknown path (not a panic).
        let bad = FramedMessage {
            magic,
            command: *b"block\0\0\0\0\0\0\0",
            payload: vec![0u8; 10],
        };
        match bad.decode().payload() {
            NetworkMessage::Unknown { command, .. } => {
                assert_eq!(command.to_string(), "block");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }

        assert!(!encode_is_cpu_heavy(&NetworkMessage::Verack));
        assert!(encode_is_cpu_heavy(&NetworkMessage::Headers(vec![])));
        // Small inv is cheap; large is heavy.
        use bitcoin::hashes::Hash as _;
        use bitcoin::p2p::message_blockdata::Inventory;
        let small = NetworkMessage::Inv(vec![Inventory::Block(
            bitcoin::BlockHash::from_byte_array([0; 32]),
        )]);
        assert!(!encode_is_cpu_heavy(&small));
        let large = NetworkMessage::Inv(vec![
            Inventory::Block(bitcoin::BlockHash::from_byte_array(
                [0; 32]
            ));
            65
        ]);
        assert!(encode_is_cpu_heavy(&large));
    }

    fn frame(command: [u8; 12], payload: Vec<u8>) -> FramedMessage {
        FramedMessage {
            magic: signet_magic(),
            command,
            payload,
        }
    }

    /// version, marker, flag, one empty input, one empty output, then a witness
    /// stack count of 4_000_000 and no element bytes.
    fn short_huge_witness_tx() -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&1i32.to_le_bytes());
        p.push(0x00);
        p.push(0x01);
        p.push(1);
        p.extend_from_slice(&[0u8; 36]);
        p.push(0);
        p.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        p.push(1);
        p.extend_from_slice(&0u64.to_le_bytes());
        p.push(0);
        p.push(0xfe);
        p.extend_from_slice(&4_000_000u32.to_le_bytes());
        p
    }

    fn assert_witness_count_too_large(command: [u8; 12], payload: Vec<u8>) {
        assert!(
            payload.len() < 512,
            "the frame must stay short so the failure is the count, not the byte cap"
        );
        match frame(command, payload).try_decode() {
            Err(crate::error::NetError::MessageTooLarge(n)) => assert_eq!(n, 4_000_000),
            other => panic!("huge witness count must be misbehavior, got {other:?}"),
        }
    }

    #[test]
    fn short_merkleblock_is_unknown_without_decoding() {
        use bitcoin::consensus::serialize;
        use bitcoin::MerkleBlock;

        let genesis = genesis_block(Network::Regtest);
        let mb = MerkleBlock::from_block_with_predicate(&genesis, |_| true);
        let payload = serialize(&mb);
        match frame(*b"merkleblock\0", payload).try_decode() {
            Ok(msg) => match msg.payload() {
                NetworkMessage::Unknown { command, .. } => {
                    assert_eq!(command.to_string(), "merkleblock");
                }
                other => panic!("a merkleblock must be ignored, got {other:?}"),
            },
            Err(e) => panic!("merkleblock must be Unknown, not {e}"),
        }

        let mut huge = vec![0u8; 80];
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.push(1);
        huge.extend_from_slice(&[0x11; 32]);
        huge.push(0xfe);
        huge.extend_from_slice(&4_000_000u32.to_le_bytes());
        assert!(huge.len() < 200);
        match frame(*b"merkleblock\0", huge).try_decode() {
            Ok(msg) => match msg.payload() {
                NetworkMessage::Unknown { command, .. } => {
                    assert_eq!(command.to_string(), "merkleblock");
                }
                other => panic!("huge bit-count must not decode, got {other:?}"),
            },
            Err(e) => panic!("huge merkleblock must be Unknown, not {e}"),
        }

        match frame(*b"zzzzzzzzzzzz", Vec::new()).try_decode() {
            Ok(msg) => assert!(matches!(msg.payload(), NetworkMessage::Unknown { .. })),
            Err(e) => panic!("unrecognized command stays Unknown, got {e}"),
        }

        match frame(*b"tx\0\0\0\0\0\0\0\0\0\0", vec![1, 0, 0, 0]).try_decode() {
            Ok(msg) => assert!(
                matches!(msg.payload(), NetworkMessage::Unknown { .. }),
                "a truncated tx is not the witness-count reject"
            ),
            Err(e) => panic!("truncated tx must stay Unknown, got {e}"),
        }
    }

    #[test]
    fn short_tx_witness_count_is_message_too_large() {
        assert_witness_count_too_large(*b"tx\0\0\0\0\0\0\0\0\0\0", short_huge_witness_tx());
    }

    #[test]
    fn short_cmpctblock_witness_count_is_message_too_large() {
        let mut payload = vec![0u8; 88];
        payload.push(0);
        payload.push(1);
        payload.push(0);
        payload.extend(short_huge_witness_tx());
        assert_witness_count_too_large(*b"cmpctblock\0\0", payload);
    }

    #[test]
    fn short_blocktxn_witness_count_is_message_too_large() {
        let mut payload = vec![0u8; 32];
        payload.push(1);
        payload.extend(short_huge_witness_tx());
        assert_witness_count_too_large(*b"blocktxn\0\0\0\0", payload);
    }

    #[test]
    fn short_block_witness_count_is_message_too_large() {
        let mut payload = vec![0u8; 80];
        payload.push(1);
        payload.extend(short_huge_witness_tx());
        assert_witness_count_too_large(*b"block\0\0\0\0\0\0\0", payload);
    }

    #[test]
    fn witness_tx_cmpctblock_and_blocktxn_still_decode() {
        use bitcoin::bip152::{
            BlockTransactions, HeaderAndShortIds, PrefilledTransaction, ShortId,
        };
        use bitcoin::consensus::serialize;
        use bitcoin::hashes::Hash as _;
        use bitcoin::p2p::message_compact_blocks::{BlockTxn, CmpctBlock};

        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::ONE,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: bitcoin::OutPoint::null(),
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: bitcoin::Sequence::MAX,
                witness: bitcoin::Witness::from_slice(&[b"abcd"]),
            }],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(1),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        };
        match frame(*b"tx\0\0\0\0\0\0\0\0\0\0", serialize(&tx))
            .try_decode()
            .expect("witness tx")
            .payload()
        {
            NetworkMessage::Tx(got) => assert_eq!(got, &tx),
            other => panic!("expected tx, got {other:?}"),
        }

        let genesis = genesis_block(Network::Regtest);
        let compact = CmpctBlock {
            compact_block: HeaderAndShortIds {
                header: genesis.header,
                nonce: 1,
                short_ids: vec![ShortId::with_siphash_keys(
                    &tx.compute_txid().to_byte_array(),
                    (1, 2),
                )],
                prefilled_txs: vec![PrefilledTransaction {
                    idx: 0,
                    tx: tx.clone(),
                }],
            },
        };
        match frame(*b"cmpctblock\0\0", serialize(&compact))
            .try_decode()
            .expect("cmpctblock")
            .payload()
        {
            NetworkMessage::CmpctBlock(got) => {
                assert_eq!(got.compact_block.prefilled_txs.len(), 1);
                assert_eq!(got.compact_block.prefilled_txs[0].tx, tx);
            }
            other => panic!("expected cmpctblock, got {other:?}"),
        }

        let blocktxn = BlockTxn {
            transactions: BlockTransactions {
                block_hash: bitcoin::BlockHash::from_byte_array([7; 32]),
                transactions: vec![tx.clone()],
            },
        };
        match frame(*b"blocktxn\0\0\0\0", serialize(&blocktxn))
            .try_decode()
            .expect("blocktxn")
            .payload()
        {
            NetworkMessage::BlockTxn(got) => assert_eq!(got.transactions.transactions, vec![tx]),
            other => panic!("expected blocktxn, got {other:?}"),
        }

        let block_payload = serialize(&genesis);
        match frame(*b"block\0\0\0\0\0\0\0", block_payload)
            .try_decode()
            .expect("genesis block")
            .payload()
        {
            NetworkMessage::Block(got) => assert_eq!(got.block_hash(), genesis.block_hash()),
            other => panic!("expected block, got {other:?}"),
        }
    }
}
