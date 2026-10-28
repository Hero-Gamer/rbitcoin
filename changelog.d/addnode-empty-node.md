Fixed

- **`addnode` rejects a blank node.** An empty or whitespace-only `node`
  was accepted, and `addnode "" add` kept the empty string on the redial
  list. As in Core
  ([bitcoin/bitcoin#35787](https://github.com/bitcoin/bitcoin/pull/35787)),
  every command now fails with `-8` `Error: Node address cannot be empty`.
