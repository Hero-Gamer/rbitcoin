Changed

- **Satisfied-block pruning judges each in-flight hash once.** The reader
  set is then replaced with that decision, so a confirm between two passes
  cannot leave the initial-block-download reader and the assign loop
  disagreeing about the next body.
