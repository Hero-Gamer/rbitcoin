Changed

- Header look-ahead keeps one tip, one hash lookup, and one rewind.
  `header.adopt` is the same file as before.
- Writing a header batch does not spend the ask window of a lane that is
  still waiting on its peer.
