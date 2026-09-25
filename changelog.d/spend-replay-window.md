Changed

- Opening a store with no `spend_durable` file checks spend annotations
  on the last 6 blocks. When they match, the marker is published at the
  tip. A mismatch still replays from genesis, and that replay logs
  progress every 10 seconds.
