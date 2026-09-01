# 0067 — Shell rendered-file removal is receipt-set transactional

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/{planner,shell,shell_action_executor}.rs`

## Context

Lifecycle-rendered Shell output is Shine-owned but may be shared by several command receipts.
Install and upgrade already replace it through `ReplaceShellRenderedFile`, but uninstall removed the
last consumer receipts first and then deleted the rendered file outside the Shell journal. A crash
could therefore leave an orphaned rendered file after receipt commit, while moving deletion before
receipt commit without a journal could leave an installed launcher with no executable source.

External live-mode launchers may also rewrite the same rendered path immediately before invocation.
Atomic file replacement prevents partial bytes, but it does not serialize that write with lifecycle
removal or explicit recovery.

## Decision

Action IR v1 adds `RemoveShellRenderedFile` for uninstall when every receipt consuming a managed
rendered path is selected, the destination is a regular file, and its canonical same-directory
rollback path is absent. The action binds:

- the exact live rendered-file hash and mode observed for removal;
- the canonical `.shine.rollback` path;
- every exact previous consumer receipt, with desired receipt absence;
- a positive per-action `receipt-committed` marker.

The Shell journal is durable before the rendered file moves to rollback. The selected receipts are
then removed atomically, the positive marker is persisted, and only the exact rollback file is
cleaned. Before that marker, explicit recovery reconstructs any missing previous receipts before
restoring the exact file. After the marker, recovery keeps the destination absent and removes only
unchanged rollback material. A changed destination, rollback, or receipt blocks all recovery
mutation. A missing rendered file needs only the existing receipt transition and creates no removal
action. An unselected consumer keeps the shared file outside the transaction.

Live rendering remains invocation-scoped and does not create a persistent journal. It acquires the
same host-provided cross-process operation lock as Shell lifecycle and recovery, refuses to run
while a Shell journal is pending, re-reads its exact manifest receipt under that lock, and retains
atomic last-known-good replacement semantics.

## Consequences

- Rendered output, launcher resources, and command receipts now recover at one coherent uninstall
  boundary.
- Receipt absence alone never authorizes rollback cleanup.
- A running live renderer cannot modify transaction or rollback state during lifecycle apply or
  recovery, and an interrupted transaction must be recovered before another live render.
- Cache uninstall, snapshot uninstall, and profile sentinel edits remain separate Phase 4 actions
  with different ownership proofs.
