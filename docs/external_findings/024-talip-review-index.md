# 024 — Talip review index

Coordinated review (2026-09-21 and 2026-09-22). This index is the status
board. Each fixed row names the regression. No reproduction steps.

| Id | Severity | Topic | Status | Regression |
|----|----------|--------|--------|------------|
| C-1 | critical | Tapleaf `0x50` rejected before commitment | fixed | `script_path_accepts_leaf_0x50_with_annex_and_0xc2` ([025](./025-tapleaf-annex-prefix.md)) |
| H-1 | high | False witness program accepted | fixed | `false_witness_program_is_eval_false` ([026](./026-false-witness-program.md)) |
| H-2 / #12 | critical | Script skip keyed by txid | open | |
| H-3 | high | Witness padding caches block hash | open | |
| #08 / #01 / #04 | critical | Header error discarded; chain wipe; work wrap | open | |
| H-5 / H-6 | high | Spend annotations after tip commit | open | |
| H-4 | high | Unbounded per-peer send queue | open | |
| #17A / C02 / L-10 | high | Signet solution bounds | open | |
| C01 | critical | Untrusted tx-count allocation | open | |
| C06 / C04 | critical | Compact partial count and one partial per peer | open | |
| C03 / C05 | high | Header explore and body-queue bytes | open | |
| C09 / C07 / C08 / C10 | critical | Public Electrum scan, subs, scripthash join | open | |
| M-2 / M-5 / M-6 / L-7 / L-8 | medium | Mempool policy and orphanage | open | |
| C12 / #09 / #10 / C13 | high | RPC body, waits, client id | open | |
| C11 / M-1 | high | Async tip-accept lifetime | open | |
| #11 / L-6 | high | I/O buffer lifetime | open | |
| C15 / #17B / #18 / L-4 | medium | Corrupt store and mmap | open | |
| M-7 | medium | Addr relay fanout | open | |
| #14 / C17 / #19 | low | Flag parity, token, inv cap | open | |
| C14 / M-4 | medium | Milestone hash and signet default | open | |
| L-1 / L-3 / L-12 | low | Local auth and datadir mode | open | |
| L-9 | low | BIP30 after the exception window | open | |
| C16 | low | Regtest BIP34 height | won't-fix | `s7_regtest_does_not_activate_bip34_early` (`bip34_height` stays rust-bitcoin's value, above 1_000_000). Signet is already height 1. Production networks are unaffected. |
| #03 / #13 / #16 | — | Witness reserved value, RBFR, admin RPC | rejected | not defects |

Owner: [`quality.md`](../quality.md) **Q-71**.
