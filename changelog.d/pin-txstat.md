Changed

- IBD parent pin no longer reads `input.loc` once per parent.
  `txstat` fees are the ones assemble already checked, so connect
  does not walk prevouts again to stamp them.
