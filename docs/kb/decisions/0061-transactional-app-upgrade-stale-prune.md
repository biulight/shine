# 0061 — App upgrade stale pruning reuses receipt-gated removal transactions

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`,
  `core/src/runtime/action_executor.rs`

## Context

`shine upgrade --prune-stale` removes App manifest entries whose logical source no longer exists in
the active Preset. The legacy upgrade path invoked the ordinary file remover directly. Its Plan
showed a stale `Remove` step but did not bind the destination, persistent backup, transaction
rollback path, removal permissions, or administrator requirement. An interruption before the
updated manifest became durable could therefore leave the resource and receipt out of sync.

The existing App removal actions already define safe receipt transitions for unchanged static Copy
files, fixed backup restoration, administrator paths, and key-owned JSON removal. Stale pruning
does not need a new rollback proof; it needs the upgrade Plan and orchestrator to use those proofs.

## Decision

An App upgrade Plan observes each stale receipt's destination, optional fixed backup, and eligible
canonical `.shine.rollback` path. `--prune-stale` produces `Remove` only while the current managed
file or declared JSON keys still match the receipt. User-modified stale state is `Preserve`, does
not derive protected-path permissions, and remains tracked.

For an exact stale static Copy or JSON receipt, Core derives the same one-action
`RemoveManagedFile`, `RemoveManagedFileWithBackup`, or `RemoveManagedJson` IR used by uninstall.
The Action IR is accepted under an Upgrade Plan only when its exact target/resource step carries
`app_stale_source_pruned`; forced removal is never admitted through stale pruning. A missing
destination needs only an atomic receipt update and does not create a journal. A present receipt
shape that cannot produce one of these exact actions blocks the Plan instead of falling back to an
unjournaled remover.

Execution writes the existing App operation journal before the first path mutation, stages or
key-prunes the resource, atomically removes the stale manifest receipt, records positive
`receipt-committed` state, and then cleans exact rollback material. Administrator authorization is
requested only for a stale action that will actually mutate a protected path. An interruption is
recovered through the existing freshly approved `shine app recover` flow; receipt absence without
the positive marker reconstructs the old receipt and restores exact rollback state.

## Consequences

- Stale pruning no longer has an unjournaled resource/receipt window for eligible static Copy and
  JSON entries.
- Plan observations and permissions now describe the removal that upgrade actually performs.
- Stale cleanup remains per target. Installing a newly introduced or renamed source after cleanup
  is a separate operation and is not part of a global upgrade transaction.
- App relocation, generator execution, and unsupported legacy receipt shapes remain separate Phase
  4 work.
