Fixed

- **A burst of unsolicited small blocks no longer trips the decode-queue
  frame cap.** `feature_versionbits_warning.py` sends 108 block messages
  before the node applies them, and the heavy-frame cap was 32, so a slow
  apply loop dropped the peer with `protocol: decode queue full`. The cap now
  fits a versionbits period (144) plus the tip-follow window, while the
  per-peer and process byte caps still bound queued block bytes.
