Added

- **`--rpc-socket PATH` (conf `rpc_socket=`).** Binds the unix JSON-RPC
  socket at PATH with mode 0660 instead of `{datadir}/rpc.sock` (0600).
  A client running as another user in rbitcoin's group, such as
  mempool's Node, can connect without reaching into the 0700 datadir.
  Implies `--rpc`. `rbitcoin-cli --rpc-socket PATH` talks to it.
  NixOS: `services.rbitcoin.rpc.socketPath` (directory created 0750).
