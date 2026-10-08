Changed

- **Packed body decodes are counted at the decode site.** The counter the
  mempool and block-proposal pins read sits on the tx table's outs decoder, so
  every per-fk decode is seen; the confirm write stage's full decoder is not
  counted.
