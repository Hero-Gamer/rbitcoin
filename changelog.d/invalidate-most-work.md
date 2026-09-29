Fixed

- **`invalidateblock` activates the most-work remaining fork.** Candidates
  were ranked by the work of their side branch alone, so an older, longer
  fork could outrank an equal-work sibling of the new tip and leave the
  node on the parent.
