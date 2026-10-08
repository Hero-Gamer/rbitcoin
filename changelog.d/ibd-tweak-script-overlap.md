Changed

- During IBD, silent-payment tweak jobs and BIP158 filter jobs run on the
  script pool in the same wave as script verification. The next batch is
  published once that wave has nothing left to claim, while the claimed
  jobs are still running.
