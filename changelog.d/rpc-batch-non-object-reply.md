Fixed

- **A non-object JSON-RPC batch element gets Core's reply.** An element such
  as `"not an object"` now answers `{"result": null, "error": {"code":
  -32600, "message": "Invalid Request object"}, "id": null}`, as in Core
  (bitcoin/bitcoin#36642). Before, it answered `Missing method` with no `id`.
