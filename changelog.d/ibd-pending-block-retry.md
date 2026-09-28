Changed

- IBD makes a block eligible for another request when its last in-flight
  peer disconnects, reports `notfound`, or loses ownership to a faster peer.
