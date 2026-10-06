Fixed

- **`submitblock` runs CheckBlock before inputs when the parent is known.**
  An equal-work sibling that fails CheckBlock is rejected and remembered.
  A second submit is `duplicate-invalid`. A block that will not connect
  used to be held as `inconclusive` when the failure was not in the cheap
  input checks. A valid sibling stays `inconclusive`. Merkle and witness
  mismatches are still not remembered.
