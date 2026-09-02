# Changelog

## 0.6.1 - 2026-09-02

- Release macOS libs now pin `MACOSX_DEPLOYMENT_TARGET` per architecture
  (10.15 for x86_64, 11.0 for arm64) so the universal `libreplicant_client.a`
  no longer triggers "was built for newer macOS version" link warnings in
  hosts targeting older deployment targets (DEV-1079).

## 0.6.0 - 2026-09-01

Document events carry an origin flag (#44). All workspace crates move to 0.6.0 together.

### Breaking

- Document events now carry their origin. `DocumentEventCallback` in
  `replicant.h` takes a new `ReplicantEventOrigin origin` argument before
  `context`, so every C/C++ callback must be updated; the JUCE wrapper's
  `onDocumentCreated`/`onDocumentUpdated`/`onDocumentDeleted` gained the same
  trailing parameter. In Rust, `SyncEvent::Document{Created,Updated,Deleted}`
  gained an `origin: EventOrigin` field, and
  `emit_document_{created,updated}_with_attribution` / `emit_document_deleted`
  take it explicitly.
- `origin` is `Local` when this client wrote the document itself and `Remote`
  when the change was applied from the server (a broadcast from another client
  or instance, or a sync pass). Events for a client's own writes are still
  emitted — separate clients over one database depend on them — so a consumer
  that only cares about changes made elsewhere must check `origin` rather than
  comparing content. (#44)

## 0.5.0 - 2026-08-28

Sync-base verification (DEV-1037). All workspace crates move to 0.5.0 together.

### Breaking

- `DocumentUpdated` payload handling changed. `ServerMessage::DocumentUpdatedResponse`
  now carries `reason`, `current_revision`, `current_content` and `current_hash`, and
  `update_document` returns the structured rejection instead of a stringified error.
  Consumers that matched on the old error string must move to the typed fields.
- New C-ABI error code `UpdateConflict = 5001` (`replicant.h`). A `hash_mismatch`
  that cannot be rebased is now surfaced to the host as this code, where it
  previously appeared as a generic error. `5xxx` means unresolved divergence:
  show it to the user and do not retry.

### Fixed

- Broadcast apply is guarded and idempotent, checks revision continuity, and
  verifies the content hash after applying.
- Targeted resync via the server's `get_document` op (own and public documents)
  with bounded retries, replacing full-sync fallbacks on a revision gap.
- An upload rejected with `hash_mismatch` rebases the queued patch and resends,
  capped at three attempts per document per session; beyond that the document
  enters a durable `Conflict` state whose content keeps following the server
  until the next local edit.
- The offline queue holds a single row per document carrying the cumulative diff
  against the stored base, so a batch of offline edits flushes as one patch
  against the revision the server actually holds.
- Server echoes of a client's own broadcasts are deferred while that document
  has an upload in flight.
- Adopting writes clear the sync queue in the same transaction as the
  compare-and-swap, so a crash cannot leave a document written but still queued.

### Testing

- `three_client_convergence_torture`: three replicas of one user run 30 seeded,
  randomized, interleaved operations with one replica toggling offline twice,
  then must converge bit-identically with the server. Set `TORTURE_SEED` to
  replay or explore an interleaving.
- The Phoenix interop harness now pins `replicant-server` at `dad45e5` by default.
