Changed

- **The API log truncates a large params body before copying it.** A
  secret that overlaps the logged prefix is still redacted. A `submitblock`
  body with no secret in that prefix is no longer copied in full on the
  work-queue thread.
