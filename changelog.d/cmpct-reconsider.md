Fixed

- **Compact fuzz oracle clears a sticky invalidate.** After a compared
  accept, Core `invalidateblock` keeps the header invalid. The next
  `submitblock` of that body is `duplicate-invalid`. The harness
  `reconsiderblock`s once and scores the second reply. A reason that
  remains is still a disagreement.
