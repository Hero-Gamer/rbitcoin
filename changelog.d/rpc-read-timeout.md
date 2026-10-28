Fixed

- RPC connections now expire stalled headers, request bodies, and idle
  keep-alive sockets after 30 seconds, freeing capacity for TCP and Unix-socket
  clients. Complete long-poll RPC requests keep their existing deadlines.
