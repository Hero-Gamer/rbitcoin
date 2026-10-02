Fixed

- A restart at the validated tip leaves IBD once no connected peer can
  extend that tip. An advertised `version.start_height` on a chain nobody
  serves no longer keeps the node out of tip mode.
- A header the walk already holds is fetched even when every peer's
  connect-time `version.start_height` is still the previous tip. A block
  announced after restart is downloaded instead of leaving the walk one
  above the confirmed tip.
