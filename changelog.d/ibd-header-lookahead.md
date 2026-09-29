Fixed

- **Header look-ahead runs until the walk reaches the tallest peer.** A download queue with room no longer switches the next header request back to the queue tail. A reply that continues the walk, and does not continue the queue, is a checkpoint and is not written. The queue still stores a reply that continues its tail while it has room.
