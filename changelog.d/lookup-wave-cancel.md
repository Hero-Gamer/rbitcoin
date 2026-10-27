Fixed

- **IBD teardown no longer waits out a whole confirm lookup wave.** A wave
  resolves up to 64k parent txids in one TipOnly batch. On a busy HDD
  datadir that batch took most of an hour, and stop, SIGTERM and a catch-up
  give-up all waited for it (`still waiting for confirm engine` for 58
  minutes in the field). The batch still runs once (one open probe, one
  sealed walk, one `create.loc` batch) and checks the confirm cancel before
  each sealed segment and each identity shot. A live sync never sets that
  flag. A cancelled wave ends the lookup thread instead of counting as a
  lookup fault.
