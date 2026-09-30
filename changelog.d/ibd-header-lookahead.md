Fixed

- **Header look-ahead runs until the walk reaches the tallest peer.** The walk keeps its own request while the queue has room, and a second request refills from the queue tail. A reply that continues the walk, and does not continue the queue, is a checkpoint and is not written. The queue still stores a reply that continues its tail while it has room.
