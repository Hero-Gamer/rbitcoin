# Mempool fee estimation (rbitcoin)

## Product opinion: 10-minute inclusion is the standard API

The **default** fee estimate this node advertises answers:

> If current mempool pressure and admit/evict/relay flows continue, what feerate
> still gets a package into the next few blocks **after about 10 minutes**?

| Surface | Role |
|---------|------|
| Electrum `blockchain.estimatefee` (default / primary) | **10-minute inclusion** |
| Esplora fee endpoints (primary) | Same |
| Optional target-depth knobs | **Near:** flow invert. **Far:** block history. Blended. |

Near (1–6 blocks) is live stock + capped admit-EMA. Far (144/504/1008) is
**what recent blocks actually paid** (per-block p10 of confirmed packages),
not min-relay and not the cheapest live chunk. Mid depths blend
`w(N)=exp(-(N-1)/6)`.

## Non-blocking vs accept (published snapshot)

Fee APIs **do not** walk the live mempool graph on every Electrum/Esplora request.

| Path | Behavior |
|------|----------|
| Accept / remove | Marks fee cache **dirty** only (no recompute on the admit critical path). |
| Refresh | Singleflight: at most one recompute; **one** `mining_chunks_best_first` under a short hub read lock, then pure math off-lock for all depths. |
| Request | Loads a published `Arc` snapshot (≤ **~1 s** stale when dirty/max-age). Histogram and frontier use the **same** published chunk list. |

This avoids fee-estimates holding the hub lock for multi-second full-pool linearizes (which previously blocked accepts and vice versa). Estimates may lag a short bound after fee spikes. Min-relay is a policy floor only. Confirm-memory clips **N=1** only.

## Engine v2 (shipped)

**Temporal flow projection** under the same APIs as v1:

1. **Clock is mempool flow, not last-block age.** Always plan block *k* at
   **T + 10 k minutes** (`capacity_wu = N × 4_000_000`). Never stretch by wall
   time since the last tip.
2. **Live stock:** mining-chunk weight strictly above candidate rate R
   (`weight_above_feerate` / frontier).
3. **Inflow EMA:** per feerate bucket, exponential moving average of admitted
   package/chunk weight per second (`FeeFlowMeter` on successful accept).
4. **Include at R** when
   `stock_above(R) + projected_inflow_above(R, min(N×600s, 600s)) ≤ 0.95 × N × 4e6`.
   Inflow horizon is capped at ~4 admit half-lives so a 150 s EMA is not
   stretched to a week.
5. **Frontier** is the marginal chunk at `N×4e6` WU. If the pool is thinner
   than N blocks, stock does **not** set a far rate (no last-chunk-as-far).
   Near depths (`w≥0.5`, N=1–5) with any live stock still answer min-relay
   (the next few blocks have room).
6. **Far / blend:** `R = w·R_flow + (1-w)·R_hist` with `w=exp(-(N-1)/6)`.
   `R_hist` is the 85th percentile (or median if <12 samples) of per-block
   p10 confirmed package feerates. Then enforce `R(1) ≥ R(2) ≥ …`.
7. **N=1** may additionally clip to the confirm-memory median. Long N does not.

**Cold start:** until the flow meter is warm (≥60 s wall and ≥32 admits),
`R_flow` is frontier, or min-relay on an under-full **near** depth with live
stock. If `R_hist` is also empty, **far** APIs return “insufficient”
(`-1` / Esplora `1.0`).

### Parameters (code constants, not env)

| Parameter | Value |
|-----------|--------|
| Block weight capacity | 4_000_000 WU |
| Seconds per planned block | 600 |
| Capacity safety margin | 95% |
| Admit EMA half-life | ~150 s |
| Inflow horizon cap | 600 s |
| Blend N0 | 6 blocks |
| Fine candidates | 100 sat/kvB steps to 10 sat/vB |
| Warm | 60 s + 32 admits |
| Bucket edges (sat/kvB) | 100…100000 (+ open top) |

### Confirm-memory / block history

Package feerates on `remove_for_block` fill a 64-sample ring (**N=1 clip**) and
a per-block p10 ring (last 1008 blocks) for `R_hist`. Process-local.

### Histogram / relayfee

Live chunk histogram remains a **stock** snapshot (transparency). Libre min
relay (0.1 sat/vB) floors any defined number.

## Template readiness (non-goal: full mining)

Frontier/chunk snapshots are shaped so a later block-template consumer can reuse
mining order. This document does **not** specify `getblocktemplate`, coinbase
construction, or witness nonces.

## Non-goals

- Full Core `estimatesmartfee` Bayesian wait-time tracker (we use per-block
  included p10s, not first-seen delay buckets)
- Full multi-node flow aggregation / peer bandwidth models
- Changing Libre min relay, dust, or full-RBF defaults
- Persisting flow meters across process restart (process-local)

## Related

- Mempool admission correctness: findings [010](./external_findings/010-mempool-confirmed-spentness.md),
  [011](./external_findings/011-mempool-structural-chain-context.md)
- Policy: `rbitcoin-consensus::policy`, OPERATOR Libre table
- Accept path: staged prepare / script_pool / commit; coalesced durable writes
