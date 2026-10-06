Changed

- A held or pending block of exactly 4,000,000 bytes is parked; one byte
  over is refused. The cap is the block count. Mempool meta is fsynced on
  the meta file.
