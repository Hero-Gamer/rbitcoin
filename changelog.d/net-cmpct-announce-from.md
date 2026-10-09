Fixed

- A reconstructed tip block is announced as `cmpctblock` before connect to
  peers who sent `sendcmpct` announce=1 and already have the parent. Peers
  we only selected as compact sources no longer receive that announce.
