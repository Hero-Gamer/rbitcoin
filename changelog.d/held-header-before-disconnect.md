Fixed

- **A side branch with a bad header no longer rewinds the tip.** Headers on a
  held branch are checked before any disconnect. A later block that claims
  enormous work without a valid proof of work is remembered invalid, and a
  heavier valid held branch can still become the tip.
