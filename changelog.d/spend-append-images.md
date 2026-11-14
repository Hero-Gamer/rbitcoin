Changed

- **A spend checkpoint appends only the pages it just composed.**
  An unchanged page stays in the generation file that already holds it.
  Schema is 29. Each commit record names that file. A schema-28 binary
  refuses the datadir, and this binary rewrites schema 22 through 28.
  Fold copies the stored image, oldest generation file first, after the
  open-edge wait is released. A failed commit directory sync leaves a
  generation file the previous commit still names. A fold before the next
  durable commit leaves that file in place. Each published image is
  composed once and written before the next page is composed. A return
  before the commit rename removes that file.
