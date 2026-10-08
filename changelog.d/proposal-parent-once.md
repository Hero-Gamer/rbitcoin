Changed

- **A block proposal decodes each confirmed parent once.** `getblocktemplate`
  proposal mode resolves a fan-out parent's fk once and decodes its packed
  outputs once per check, not once per spending input.
