Fixed

- JSON-RPC accepts a request body up to 32 MiB, so `submitblock` and a
  `getblocktemplate` proposal can carry a block larger than 1 MB. REST
  stays at 2 MiB. A missing or wrong RPC credential is still rejected
  before the body is read.
