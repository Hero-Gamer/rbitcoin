Fixed

- **`tx.head` no longer drops entries after a gap in Class A:** a confirm
  write that appends its bodies and then rejects on the planned-fk check
  leaves bodies the head never indexes. Segments rolled on entry count while
  the seal re-reads an fk range, so such a gap made the next seal cover the
  wrong range and lose that many real entries. IBD then halted on
  `missing prevout (leftover … leftover_n=0)` at every restart. Segments now
  own an fk span. Heads that already lost entries need `store/tx.head` moved
  aside once, so open rebuilds it from Class A (#843).
