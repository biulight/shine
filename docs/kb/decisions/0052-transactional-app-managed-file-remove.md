# 0052 — App managed-file removal commits through receipt absence

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

The Phase 4 App update action retains previous managed bytes at a same-directory transaction path
until a replacement receipt is durable. Uninstall has the inverse receipt boundary: it begins with
an exact ownership receipt and commits only when that receipt has been durably removed. Deleting the
destination before the manifest save can otherwise lose managed bytes if the process is interrupted,
while copying those bytes into the journal would enlarge the persistent plaintext surface.

Persistent `.shine.bak` restoration, forced removal of user-modified content, JSON merge and
administrator paths have distinct state and authorization requirements. Combining them with the
first removal action would make receipt absence ambiguous and weaken the narrow recovery proof.

## Decision

Action IR v1 adds `RemoveManagedFile` for one unchanged, receipt-owned, unprivileged static Copy
whose persistent backup is absent. The action binds its destination, canonical same-directory
`<name>.shine.rollback` path, prior mode and content hash, but contains no file bytes.

The executor revalidates the approved uninstall Plan, exact receipt, regular-file kind, mode, hash,
and absent unclaimed rollback path under the host operation lock. It then writes the prepared
journal, renames the destination to rollback material, marks the action applied, and returns to App
lifecycle orchestration. The lifecycle removes and durably saves the receipt, then records a
`receipt-committed` journal state before commit removes the unchanged rollback material and journal.
Receipt absence without that positive marker is ambiguous and cannot authorize cleanup.

Recovery distinguishes the receipt boundary:

| Journal/receipt | Destination | Rollback | Recovery |
|---|---|---|---|
| applied + exact original receipt | original | missing | clear journal; mutation did not start |
| applied + exact original receipt | missing | exact original | restore rollback to destination |
| applied + absent/unclaimed receipt | missing | exact original | restore receipt, then destination |
| applied + absent/unclaimed receipt | original | missing | restore receipt |
| receipt-committed + absent/unclaimed receipt | missing | exact original | remove rollback |
| receipt-committed + absent/unclaimed receipt | missing | missing | clear journal |

Any other receipt, path kind, mode, hash, or occupied destination blocks recovery and preserves all
state. Receipt absence counts as commit only when the journal has the durable positive marker and no
manifest entry claims the action source, destination, or rollback path. Absence without the marker,
including after a receipt-save/marker-write interruption, cannot authorize cleanup; freshly approved
recovery reconstructs the exact payload-free old receipt and rolls the file back. Other receipt or
path combinations block and preserve rollback material.

At adoption, forced, backup-restoring, administrator, JSON merge, relocation, stale-prune and
upgrade-internal removals remained on their existing executors. [ADR 0053](0053-transactional-app-backup-restoring-remove.md)
later adds the narrower persistent-backup contract and
[ADR 0054](0054-transactional-forced-app-managed-file-remove.md) adds a distinct forced-removal
contract; the other cases remain outside this action.

## Consequences

- An interrupted ordinary App uninstall can restore the exact managed file while its receipt still
  exists, without serializing its bytes.
- Once receipt removal and its journal marker are durable, recovery never recreates an unowned file;
  it removes only unchanged transaction rollback material.
- The predictable rollback path may temporarily contain sensitive managed configuration and is
  guarded by kind, mode, hash and ownership checks.
- This first uninstall action intentionally does not cover persistent user backup restoration,
  explicit force semantics or privileged mutations; ADRs 0053 and 0054 add separate actions for the
  first two cases without weakening this action's proof.
