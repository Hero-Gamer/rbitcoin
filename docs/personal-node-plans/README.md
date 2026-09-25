# Personal wallet-node plans

**Not scheduled.** Implement a numbered file only when the user names that
plan. Do not start the next file because it is listed here.

Implementation plans for a **home node that cannot open clearnet listen ports**:
P2P over Tor SOCKS / I2P SAM / CJDNS, Electrum and Esplora on onion (and I2P
when SAM can forward), P2P inbound on a Tor onion, local tx announce on a
short-lived isolated Tor circuit.

Live operator flags and COMPAT rows stay in [`OPERATOR.md`](../../OPERATOR.md)
and [`COMPAT.md`](../../COMPAT.md) **when a numbered plan ships**. This directory
is the plan group only. Do not copy step lists into [`quality.md`](../quality.md)
until a slice is scheduled (same pattern as [`peer-clients.md`](../peer-clients.md)
ranked items).

Each numbered file is one plan: [`how-we-plan.md`](../how-we-plan.md#agent-contract)
(Contract / Red / Green / Refactor / Verify). Commands:
[`.agents/skills/ship-pr/SKILL.md`](../../.agents/skills/ship-pr/SKILL.md).
Do not copy the cycle here.

## Operator outcome

A node on CGNAT / no forwarded TCP still:

1. Syncs and talks P2P through Tor SOCKS, I2P SAM, and/or CJDNS.
2. Accepts **P2P inbound on a Tor onion** (and I2P/CJDNS overlays) without an
   ISP port forward.
3. Serves **Electrum and Esplora** on Tor onion (plain TCP; no in-binary TLS).
4. Announces locally submitted txs on a **short-lived isolated Tor circuit**,
   not on the standing peer set.
5. Optionally **prunes witness** below a 288-height window. Kept heights are
   one file each plus a RAM cache (`0` means files only). The node advertises
   BIP159 `NODE_NETWORK_LIMITED` ([09](./09-seqsigwit-prune.md)). Unpruned nodes
   keep `seqsigwit.body`.

## Constraints (all numbered files)

- Tor backend is **system `tor`** (SOCKS + control port). Not Arti-in-process.
- Fake SOCKS / fake Tor control / fake I2P SAM under `/tmp`. **No** agent-VM
  mainnet. **No** live Tor, I2P, or cjdns router in product PRs or default
  `cargo test`. The labeled/nightly overlay job is the live-daemon oracle
  ([`overlay-functional.md`](../overlay-functional.md)).
- Prefer zero new crates (hand-rolled SOCKS5, control protocol, SAM). Any crate
  must pass `cargo deny` (musl operator binary).
- Operator CLI is kebab (`--only-net`, `--i2p-sam`, `--cjdns-reachable`,
  `--listen-onion`). Conf is snake_case (`only_net=`). Core concatenated names
  (`-onlynet`, `-i2psam`, `-cjdnsreachable`, `-listenonion`) stay the functional
  shim only — do not advertise them on `rbitcoin-node`.
- Named `ibd: perf` timer only if lookup / load / scripts / write (or a sidecar
  the write thread joins) grows — unlikely here.
- One product PR per numbered file. Each step is one cycle turn, committed
  before the next step starts, only after **local CI except coverage**
  ([`how-we-plan.md`](../how-we-plan.md#agent-contract); commands in
  [ship-pr](../../.agents/skills/ship-pr/SKILL.md)). Coverage and native
  `windows` / `macos` stay GitHub Actions. Optional holistic refactor after
  the last step, then push and poll.
- Each product PR updates [`nix/modules/rbitcoin.nix`](../../nix/modules/rbitcoin.nix)
  with **first-class options** for that slice (not `extraArgs` as the only
  path) and pins argv in
  [`nix/tests/nixos-module-eval.nix`](../../nix/tests/nixos-module-eval.nix).
  Eval is required CI. When systemd `After`/`Wants`, users/groups, firewall,
  or the VM start argv change, extend
  [`nixos-module-runtime.nix`](../../nix/tests/nixos-module-runtime.nix)
  and label the PR **`nixos-module-runtime`**. Do not start live Tor / i2pd /
  cjdns in those tests.

## Out of this group

- IBD bootstrap snapshots. BIP157/158 basic filters ship as `--block-filter-index`.
- In-binary Electrum TLS / rustls / fingerprint pairing; scripthash allowlist;
  `--personal` preset / `getwalletconnect`; **Q-63** as a TLS story.
- UPnP / NAT-PMP; in-process Arti; Core wallet RPC; Dandelion++ stem/fluff
  (see [06](./06-ephemeral-tor-broadcast.md)); public Electrum `hosts` gossip
  on P2P.
- Core `-prune` of whole `blk` files / dropping `txout`+headers (09 is
  **seqsigwit-only**).

## Dandelion++ is not plan 06

**Dandelion++** is stem/fluff P2P relay anonymity. Core never merged it. It is
not this group.

**Short-lived Tor broadcast** (plan 06): after local submit, new SOCKS
credentials, BIP324 handshake, send `tx`, disconnect. Standing peers must not
INV that tx.

## Why several PRs

Today: Electrum/Esplora are plain TCP ([`OPERATOR.md`](../../OPERATOR.md);
parked **Q-63** is TLS+onion in the binary — this group takes onion without
TLS). Dial is `TcpStream::connect` on `SocketAddr` only
([`peer_io.rs`](../../crates/rbitcoin-net/src/ibd/peer_io.rs),
[`service.rs`](../../crates/rbitcoin-net/src/service.rs)). `learn_addrv2` drops
onion/I2P (`AddrV2::socket_addr()` fails)
([`peers.rs`](../../crates/rbitcoin-net/src/peers.rs)). `max_inbound` must be
`>= 1` ([`config.rs`](../../crates/rbitcoin-node/src/config.rs)). P2P always
binds (default `127.0.0.1:<p2p>`) in
[`run.rs`](../../crates/rbitcoin-node/src/run.rs). `server.features.hosts` is
`{}`. No SOCKS, no Tor control, no I2P SAM.

SOCKS cannot create a hidden service. Wallet and P2P reachability without a
forwarded TCP port is Tor control `ADD_ONION` (03 / 05 / 07), I2P SAM STREAM
FORWARD (04 / 05), or CJDNS overlay bind (08).

P2P onion inbound does **not** need Arti. It is Core `-listenonion` (operator
`--listen-onion`): `ADD_ONION` to a loopback P2P accept socket.

## File map (product PR order)

Execute **00 → 09** as separate topic branches (one plan, one PR). Pause after
any PR. **07** can land as soon as **01 + 03** exist. **08** can land as soon as
**02** exists. **09** is independent of 00–08 (store + service bits).

| File | Story |
|------|--------|
| [00-socks-proxy.md](./00-socks-proxy.md) | `--proxy` / `--onion` SOCKS5, remote DNS, stream isolation |
| [01-listen-off.md](./01-listen-off.md) | `--listen=0`, `--max-inbound 0`, no self-announce / discover |
| [02-onion-addrman.md](./02-onion-addrman.md) | `NetAddr` + onion persist + `--only-net` + SOCKS dial of `.onion` |
| [03-tor-control-hs.md](./03-tor-control-hs.md) | Control AUTH + `ADD_ONION` helper; Electrum hidden service |
| [04-i2p.md](./04-i2p.md) | I2P SAM v3, `--only-net=i2p`, optional incoming |
| [05-wallet-onion.md](./05-wallet-onion.md) | Esplora onion HS; Electrum `features.hosts`; I2P wallet forward if cheap |
| [06-ephemeral-tor-broadcast.md](./06-ephemeral-tor-broadcast.md) | Isolated SOCKS one-shot for locally submitted txs |
| [07-p2p-onion-inbound.md](./07-p2p-onion-inbound.md) | P2P `--listen-onion`: ADD_ONION → loopback BIP324 accept |
| [08-cjdns.md](./08-cjdns.md) | BIP155 CJDNS, `--cjdns-reachable`, `--only-net=cjdns` |
| [09-seqsigwit-prune.md](./09-seqsigwit-prune.md) | Unpruned `seqsigwit.body`, or a 288-height window (`seqsigwit.window/{height}.bin` + RAM; `0` = files only); BIP159 `NETWORK_LIMITED`; partial Esplora JSON, refuse wire |

NixOS first-class options (same PR as the flags). Label **`nixos-module-runtime`**
when the row says runtime:

| File | `services.rbitcoin` | Runtime label |
|------|---------------------|---------------|
| 00 | `proxy`, `onionProxy`, `proxyRandomize` (default true) | only if `After=tor` |
| 01 | `p2p.listen` (default true), `p2p.maxInbound` | if listen-off changes VM `ExecStart` |
| 02 | `onlyNet` | eval |
| 03 | `tor.control`, cookie, `electrum.hiddenService`; `After=tor.service` | **yes** |
| 04 | `i2p.sam`, `i2p.acceptIncoming`; `After=i2pd.service` | **yes** |
| 05 | `esplora.hiddenService` (reuse 03 control) | eval unless new systemd deps |
| 06 | none if `--proxy` already implies isolated broadcast; document on `proxy` | eval |
| 07 | `p2p.listenOnion`; compose with `p2p.listen = false` | **yes** |
| 08 | `cjdns.reachable`; listen on cjdns; `After=cjdns.service` | **yes** |
| 09 | `pruneSeqSigWit` (cold dir already exists) | eval |

## Follow-ups (not scheduled here)

Arti-in-process; in-binary TLS (remainder of Q-63); SH allowlist; `--personal`
preset; Dandelion++ stem/fluff; snapshots; BIP157; xpub allowlist; Core-style
full-block prune (txout/headers).
