# 0062 — App static Copy relocation is one receipt transaction

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/planner.rs`,
  `core/src/runtime/action_executor.rs`, `core/src/runtime/app.rs`

## Context

ADR 0025 defines one App manifest destination per logical source and allows upgrade to relocate an
unchanged managed file when its effective destination changes. The legacy executor wrote the new
destination first, removed or restored the old destination second, and persisted the replacement
receipt later. An interruption could therefore leave both copies, no managed copy, or a restored
old user file plus an old receipt. It also carried the old persistent backup path into the new
receipt even though that backup had already been restored or no longer belonged to the new path.

Existing create, update, and removal actions cannot be composed safely for relocation: create
expects receipt absence, removal commits through receipt absence, while relocation atomically
replaces one receipt for the same source. A distinct action must bind both sides of that transition.

## Decision

Add `RelocateManagedFile` for an approved Upgrade Plan whose exact target/resource carries
`app_destination_relocated`. The first slice covers static Copy without force or generator output.
It binds the exact previous receipt, old destination and optional canonical persistent backup, old
same-directory rollback path, absent new destination, desired content hash, old/new environment
identity flags, and both administrator identities. An old destination that is already missing is
supported only when no persistent backup remains to restore.

Execution writes the App operation journal, moves an existing old managed file to rollback,
restores an exact old persistent backup when present, writes the new managed file, and marks the
action applied. The caller then atomically replaces the manifest receipt with a new receipt whose
backup is empty. Only after that receipt is durable may commit remove exact old rollback material.

Explicit `shine app recover` accepts only states produced at those boundaries. Before the new
receipt, it removes an unchanged newly written destination, returns a restored user file to the old
backup path when necessary, and restores the exact old managed file. After the new receipt, it
preserves both final destinations and removes only unchanged old rollback material. Any changed
kind, mode, hash, receipt, backup, destination, or rollback path blocks recovery. Protected old and
new paths use their separately persisted administrator identities under the shared operation lock.

## Consequences

- Static Copy relocation no longer has an unjournaled two-destination/one-receipt window.
- Successful relocation cannot retain the old backup identity on the new manifest entry.
- A destination or rollback path appearing after Plan review fails snapshot revalidation before
  mutation.
- JSON merge relocation does not inherit this whole-file proof; its separate key-owned
  two-destination recovery contract is defined by
  [ADR 0063](0063-transactional-app-json-relocation.md).
