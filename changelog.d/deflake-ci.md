Fixed

- **A same-slot store probe and a full RPC work queue no longer fail when the schedule is unlucky.** The probe keeps drawing until the mixed key shares the page, and a full work queue answers HTTP 503. A live follow session counts an unknown BIP324 short id as `*other*` and stays connected.
