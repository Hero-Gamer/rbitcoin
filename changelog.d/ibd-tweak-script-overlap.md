Changed

- During IBD, silent-payment tweak jobs and BIP158 filter jobs run on the
  script pool beside script verification. Filters are one job each and are
  claimed one at a time; tweaks are ranges of at most 32 transactions. The
  next batch is published once both waves have nothing left to claim, while
  the claimed jobs are still running.
