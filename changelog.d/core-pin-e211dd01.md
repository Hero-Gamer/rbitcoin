Changed

- **Core's tests now run from Bitcoin Core master `e211dd01`.** It is the
  merge of bitcoin/bitcoin#36869 and brings `interface_rpc.py`'s check of
  the reply to a non-object batch element (bitcoin/bitcoin#36642) into the
  nightly functional run. No functional scripts were added or removed, and
  the consensus vectors are unchanged.
