Fixed

- **A Class C shrink flush keeps a dirty prefix below the new length.**
  The file shortens after that prefix is written. A truncate that did not
  store into the kept prefix only shortens the file.
