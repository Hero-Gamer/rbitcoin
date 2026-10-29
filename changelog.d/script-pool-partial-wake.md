Fixed

- **Script-pool wake check retries a partial unpark.** The coverage job
  failed `steal_publish_wakes_idle_workers` when a worker left `park`
  during the publish gap and claimed one chunk of a two-chunk wave. The
  publish then unparked the other worker, and the check treated that 1
  as a miss. It already retried a total miss. It retries a partial one
  too. The test job on the same commit passed.
