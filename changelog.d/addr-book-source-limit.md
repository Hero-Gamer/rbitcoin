Security

- **One sender can no longer replace the whole address book.** In a full
  book, a source netgroup (the sending peer's IPv4 /16, IPv6 /32, or ASN
  with `--asmap`) that already taught 512 of the 8,192 rows, the 1/16
  share Core gives one source group, cannot evict another row; its new
  addresses are dropped. Before, each new inbound connection could send
  one address and evict one untried row, so one IP replaced every untried
  row in about six minutes. A book with room still takes every address
  from a single peer, so a seed or addr-fetch reply can fill an empty book,
  and those rows stay. The limit is per group, so about 16 groups (one IP
  in each of 16 /16s) can still replace every untried row between them;
  Core keeps an untried address with one reference whatever the number of
  sources. Sources are kept in memory only: rows loaded from the `peers`
  file after a restart have none, and their groups start uncharged.
