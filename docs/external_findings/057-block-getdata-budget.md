# 057 — Block serving stops when the send budget is over

**Severity:** medium
**Status:** fixed
**Found by:** Stephan Livera, 2026-10-02 (M1)

Block getdata could queue more data after the per-peer send budget was already over. Serving pauses once that budget is over and resumes when the writer drains ([090](./090-getdata-serve-pause.md)).

**Regression:** `rbitcoin-net` `getdata_over_send_budget_waits_for_writer`.
