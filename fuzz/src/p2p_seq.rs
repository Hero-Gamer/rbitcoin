//! Stateful P2P sequence. Bytes that do not start with `0xA5` stay the
//! one-byte `{ping, block, headers, skip}` map. `0xA5` starts tagged steps
//! of `tag || u16le len || payload`, at most eight.

pub const P2P_SEQ_IR: u8 = 0xA5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P2pSeqKind {
    Ping,
    Block,
    Headers,
    Tx,
    GetHeaders,
    Cmpct,
    Blocktxn,
    FeeFilter,
    Inv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2pSeqStep {
    pub kind: P2pSeqKind,
    pub skip: bool,
    pub payload: Vec<u8>,
}

fn legacy_step(b: u8) -> P2pSeqStep {
    match b % 4 {
        0 => P2pSeqStep {
            kind: P2pSeqKind::Ping,
            skip: false,
            payload: Vec::new(),
        },
        1 => P2pSeqStep {
            kind: P2pSeqKind::Block,
            skip: false,
            payload: Vec::new(),
        },
        2 => P2pSeqStep {
            kind: P2pSeqKind::Headers,
            skip: false,
            payload: Vec::new(),
        },
        _ => P2pSeqStep {
            kind: P2pSeqKind::Ping,
            skip: true,
            payload: Vec::new(),
        },
    }
}

fn kind_from_tag(tag: u8) -> Option<P2pSeqKind> {
    Some(match tag {
        0 => P2pSeqKind::Ping,
        1 => P2pSeqKind::Headers,
        2 => P2pSeqKind::Block,
        3 => P2pSeqKind::Tx,
        4 => P2pSeqKind::GetHeaders,
        5 => P2pSeqKind::Cmpct,
        6 => P2pSeqKind::Blocktxn,
        7 => P2pSeqKind::FeeFilter,
        8 => P2pSeqKind::Inv,
        _ => return None,
    })
}

fn push_skip(out: &mut Vec<P2pSeqStep>) {
    out.push(P2pSeqStep {
        kind: P2pSeqKind::Ping,
        skip: true,
        payload: Vec::new(),
    });
}

pub fn parse_p2p_sequence(data: &[u8]) -> Vec<P2pSeqStep> {
    let mut out = Vec::new();
    if data.first().copied() != Some(P2P_SEQ_IR) {
        for &b in data.iter().take(8) {
            out.push(legacy_step(b));
        }
        return out;
    }
    let mut rest = &data[1..];
    while out.len() < 8 && !rest.is_empty() {
        let tag = rest[0];
        if rest.len() < 3 {
            push_skip(&mut out);
            break;
        }
        let len = u16::from_le_bytes([rest[1], rest[2]]) as usize;
        if rest.len() < 3 + len {
            push_skip(&mut out);
            break;
        }
        let payload = rest[3..3 + len].to_vec();
        rest = &rest[3 + len..];
        match kind_from_tag(tag) {
            Some(kind) => out.push(P2pSeqStep {
                kind,
                skip: false,
                payload,
            }),
            None => push_skip(&mut out),
        }
    }
    out
}

/// Core's header hashes must be ones the hub already has.
pub fn header_sequence_agrees(
    returned: &[bitcoin::BlockHash],
    known: &[bitcoin::BlockHash],
) -> bool {
    returned.iter().all(|hash| known.contains(hash))
}

pub fn p2p_sequence_ping_comparisons(data: &[u8]) -> u32 {
    parse_p2p_sequence(data)
        .iter()
        .filter(|s| !s.skip && s.kind == P2pSeqKind::Ping)
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;

    #[test]
    fn two_ping_steps_record_two_comparisons() {
        assert_eq!(p2p_sequence_ping_comparisons(&[0, 0]), 2);
        assert_eq!(parse_p2p_sequence(&[0, 0]).len(), 2);
    }

    #[test]
    fn malformed_step_is_skip_not_death() {
        let steps = parse_p2p_sequence(&[3]);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].skip);
        assert_eq!(p2p_sequence_ping_comparisons(&[3]), 0);
    }

    #[test]
    fn legacy_zero_bytes_stay_pings() {
        let steps = parse_p2p_sequence(&[0, 0]);
        assert_eq!(steps.len(), 2);
        assert!(steps.iter().all(|s| s.kind == P2pSeqKind::Ping && !s.skip));
    }

    #[test]
    fn tagged_tx_payload_parses() {
        let raw = [P2P_SEQ_IR, 3, 1, 0, 0xab];
        let steps = parse_p2p_sequence(&raw);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kind, P2pSeqKind::Tx);
        assert!(!steps[0].skip);
        assert_eq!(steps[0].payload, vec![0xab]);
    }

    #[test]
    fn truncated_tagged_length_is_a_skip() {
        let steps = parse_p2p_sequence(&[P2P_SEQ_IR, 3, 0x10, 0x00]);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].skip);
    }

    #[test]
    fn header_sequence_must_be_known_to_the_hub() {
        let known = bitcoin::BlockHash::from_byte_array([1; 32]);
        let other = bitcoin::BlockHash::from_byte_array([2; 32]);
        assert!(header_sequence_agrees(&[], &[known]));
        assert!(header_sequence_agrees(&[known], &[known]));
        assert!(!header_sequence_agrees(&[other], &[known]));
    }

    #[test]
    fn tagged_sequence_stops_at_eight() {
        let mut raw = vec![P2P_SEQ_IR];
        for _ in 0..10 {
            raw.push(0);
            raw.extend_from_slice(&0u16.to_le_bytes());
        }
        assert_eq!(parse_p2p_sequence(&raw).len(), 8);
    }
}
