Changed

- **The full-queue RPC check calls the handler with an empty work queue.** A request still gets HTTP 503 and `Work queue depth exceeded`. The handler no longer pauses a call for that check.
