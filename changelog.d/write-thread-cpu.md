Changed

- Confirm write collects each block's spend absolute offsets once, reuses
  the structural scratch and the in-batch double-spend set, and looks up
  create heights by foreign-key span when every block in the batch is
  contiguous. The tip event carries the wire header already validated on
  the write path.
