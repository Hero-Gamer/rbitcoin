Changed

- Header look-ahead and the download-queue refill use one peer, the
  lowest time to a first block byte among peers that have not failed the
  walk. Other peers download blocks. A short reply that does not extend
  the candidate moves that reservation. A block announcement from another
  peer is one challenge: the reservation moves only when the reply beats
  the candidate. One missed header ask moves the reservation; a second
  miss disconnects. Less work does not disconnect.
