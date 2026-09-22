# rbitcoin-esplora

Esplora-compatible REST and wallet-scoped WebSocket. Depends on query, store,
net, and mempool. Electrum is a sibling crate (`rbitcoin-electrum`); both
need `--sh-index`.

## Read first

Open the row that matches the change. Leave the other owners closed.

| Change | Read |
|--------|------|
| Shipped HTTP / WS surface | [`COMPAT.md`](../../COMPAT.md) |
| Flags, listen, SH tradeoffs | [`OPERATOR.md`](../../OPERATOR.md) |
| `/internal/*` and unix listen | [`COMPAT.md`](../../COMPAT.md), [`OPERATOR.md`](../../OPERATOR.md) |
| JSON-RPC overlap (broadcast, unix `rpc.sock`) | [`docs/rpc.md`](../../docs/rpc.md) |

## Where

- Router and listen: `src/server.rs`
- Handlers: `src/handlers.rs`
- electrs `/internal/*`: `src/internal.rs`
- Tx JSON: `src/tx_json.rs`
- WS: `src/ws.rs`

## Rules here

- Address-prefix and Liquid stay 404. Do not add `/api/v1/` catalogue routes.
- Last-1 GET + last-bulk POST (16 MiB packed, including last_sh) `sh_join` per `X-Rbitcoin-Client` (unix/loopback). Unbounded process LRU stays **X-M3**. Sticky joins stay Electrum TCP.
- Wallet WS (`/v1/ws`, `/ws`): ping/init/stop, track-address snapshot via `scripthash_mempool`, RBF `address-removed-transactions`, `want: stats` from the fee snapshot. Node `/api/v1/ws` stays out.
- Do not grow a `*_for_test` backdoor. Tests drive the shipped route.

## Verify

`cargo test -p rbitcoin-esplora --lib`
