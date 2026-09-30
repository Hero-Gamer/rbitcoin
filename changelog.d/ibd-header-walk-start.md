Fixed

- Header look-ahead starts at the stored tip on the first ask. A solicited
  continuation of that tip is a checkpoint, so the walk runs ahead of the
  download queue and the `ibd: headers` line is logged.
