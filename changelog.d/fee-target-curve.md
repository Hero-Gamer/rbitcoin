Fixed

- **Fee estimates answer confirm targets between the computed depths.**
  The estimator still prices 1, 2, 3, 4, 5, 6, 10, 20, 144, 504, and 1008
  blocks. Any other target from 1 to 1008 is the straight line between the
  computed rates on either side, in whole sat/kvB, or the last defined rate
  past the far end. `estimatesmartfee`, `estimaterawfee`, and Electrum
  `blockchain.estimatefee` use that curve. Esplora `/fee-estimates` includes
  each integer from 1 through 25, plus 144, 504, and 1008, when that target
  has a rate.
