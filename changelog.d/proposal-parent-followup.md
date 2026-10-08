Changed

- **Packed body decodes are counted at the decode site.** The counter the
  mempool and block-proposal pins read sits on the tx table's outs decoder, so
  every per-fk decode is seen; the confirm write stage's full decoder is not
  counted.
- **Txid and spender resolution no longer decode the body.** Resolving a txid
  to its row, probing whether an output is spent, and recording a spend read
  the `txid.body` identity only; the packed decode runs only for a reader that
  needs the record.
