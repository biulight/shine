# ADR 0144: Share ordered App recovery observations

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Five App recovery planning branches repeat the same destination/rollback reads and snapshot
registration: file update, ordinary/forced file removal, JSON merge and JSON removal. Share this
sequence in `recovery_plan.rs`, restricted to `FileSystemObservationHost`.

Read the destination first, then rollback, before registering either observation. Preserve the
`destination:<action_id>` and `rollback:<action_id>` labels and existing byte/mode/kind identities.
Keep error short circuits and partial digest registration unchanged. Forced removal observes its
optional persistent backup only after this pair succeeds. Three-file backup removal and relocation
retain their own observation sequences; recovery apply still performs fresh live reads.

Receipt conflict checks, action-specific assessments, permissions, Plan/approval validation,
lifecycle and privileged locks, receipt markers, journal formats and reverse recovery execution
remain with their existing owners. No observation result is cached across review and apply.

Preset migration rendering also reuses `preset_report::count_phrase`; remove its identical local
copy while retaining exact singular/plural wording. Historical migration and receipt readers remain
because production callers still use them. GPG/age recipient validators stay separate because their
whitespace acceptance differs.

## Coverage and maintenance

Keep fixed state digests and destination-before-rollback read traces for all five recovery actions,
plus the normal forced-removal backup read after that pair. Retain the lifecycle/privileged lock
wait matrix: changes to destination, rollback or receipt bytes (including formatting-only changes)
must invalidate approval without mutating resources, receipts or the journal.

Do not retain the artificial duplicate-label matrix or its expected partial digest construction.
The production entry validates the journal and unique Action IDs before creating a fresh recovery
state. A manually prepopulated conflicting state bypasses that entry, and a failed planning call's
partial digest is not consumed. Duplicate observation rejection belongs to the existing snapshot
builder test; normal read order and fixed identities remain consumer tests. This removes test-only
implementation coupling without changing production error order or recovery behavior.

Existing recovery tests retain missing/non-file resources, modified bytes/modes, receipt gaps and
commit markers, stale approval and explicit rollback. Native platform execution and external
administrator operations require evidence beyond virtual Host tests.
