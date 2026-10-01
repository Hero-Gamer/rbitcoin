Fixed

- IBD enters tip mode once the proven header walk is at the confirmed tip
  and every peer that advertised a taller `version.start_height` has failed
  to extend it. One peer advertising a height no chain has no longer keeps
  the node in IBD after the last block confirms.
