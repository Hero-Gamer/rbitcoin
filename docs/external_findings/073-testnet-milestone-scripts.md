# 073 — Omitted testnet milestone checks every script

**Severity:** medium
**Status:** fixed
**Found by:** Stephan Livera, 2026-10-02 (M6)

An omitted milestone on testnet was height-only, so script checks were skipped below that height on any chain. The default is now anchored to Bitcoin Core's testnet3 assumeutxo block at height 2_500_000 and Core's testnet3 minimum chain work. Scripts run unless that header path and work floor match. An explicit `--milestone HEIGHT` stays height-only. Omitted mainnet stays anchored the same way. NixOS `services.rbitcoin.milestone` is unset by default and passes `--milestone` only when set.

**Regression:** `rbitcoin-node` `p3_default_milestone_heights`, `testnet3_default_milestone_is_anchored`, `operator_conf_and_argv`.
