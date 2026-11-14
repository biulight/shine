# 0168 — Profile repair can retain an exact unchanged receipt

- **Status**: accepted
- **Date**: 2026-11-14

## Problem

Switching Bash/Zsh or repairing a loader can change owned blocks without changing the item's
activation state. Two invocations captured in the same second also share `updated_at`. Rejecting
all identical previous/desired receipts makes an approved profile operation fail after generated
profile composition. Advancing time in a test hides this failure.

## Decision

Keep exact receipt identity and the positive `ReceiptCommitted` journal marker. Only a typed
`ReconcileSysProfileBlocks` action for `sys/profile` may bind the same nonempty, non-managed
receipt on both sides, with the receipt OS matching the action OS. Model this as `Unchanged`,
not `Desired` or missing state. Other Sys actions still require distinct receipt states.
Do not fabricate timestamps, receipts, or activation changes merely to distinguish the boundary.

Before the durable commit marker, recovery restores owned blocks and requires original rollback
material wherever mutation occurred. After the marker, recovery keeps desired owned blocks and
allows already-cleaned rollback paths to be absent. Any remaining rollback file must still match
its exact identity. Both paths reject conflicting receipts and modified owned blocks, while
preserving unrelated user content. A receipt-save or marker-save failure never counts as commit.

No fields or schema versions change. Older binaries reject an interrupted journal containing equal
receipts before mutation; recover with the binary that supports this boundary before downgrading.
Generated active/base/new/merge composition remains explicitly outside transactional rollback.

## Coverage

Arch and Ubuntu fixtures keep time frozen, switch shells, and inject receipt-save, commit-marker
and final cleanup failures. Negative cases change receipts, owned blocks, or rollback availability,
and reject unchanged managed receipts. Native Arch fixtures no longer advance the clock to pass.
The planner also declares `.new`/`.merge` removal and the backup directory only when force-profile
is requested; random backup filenames stay out of Plan fingerprints.
