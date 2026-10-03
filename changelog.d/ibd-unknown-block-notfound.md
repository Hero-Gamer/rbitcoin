Fixed

- **A peer that asks for a block this node has never seen gets `notfound`.**
  Silence held that getdata until the 30s stall floor, and the next assign
  pass asked the same peer again, so a lighter fork peer could pin a heavier
  tip for two stall waits. The hash is remembered for that peer and is not
  requested from them again this IBD.
