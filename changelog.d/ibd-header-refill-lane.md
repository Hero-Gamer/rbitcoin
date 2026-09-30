Fixed

- **Header download refills while the walk runs ahead.** The walk and the queue each have a header request, a peer, and a miss count. Two peers are asked for both. One peer keeps walking until the queue falls below 16,384 headers, then refills from the stored tail. An empty queue is rebuilt from headers still on the work path before that refill. A reply is classified by the hash it builds on: a walk continuation is a checkpoint while the walk is ahead, and a stored header once the walk has caught that top.
