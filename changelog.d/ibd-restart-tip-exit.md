Fixed

- A restart at the validated tip leaves IBD once no connected peer can
  extend that tip. An advertised `version.start_height` on a chain nobody
  serves no longer keeps the node out of tip mode.
