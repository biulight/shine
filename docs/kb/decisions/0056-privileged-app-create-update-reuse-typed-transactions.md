# 0056 — Privileged App create and update reuse typed transactions

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`, `core/src/runtime/host.rs`

## Context

ADRs [0048](0048-separate-action-ir-and-explicit-recovery.md),
[0050](0050-backup-aware-app-creation-recovery.md), and
[0051](0051-transactional-app-managed-file-update.md) define recoverable static Copy creation and
update, but initially admit only unprivileged destinations. Administrator App files therefore still
used the legacy executor: it held the administrator lock while writing, but could release that lock
and lose the previous bytes before the matching App receipt was durable.

ADR [0055](0055-privileged-app-removal-reuses-typed-transaction.md) established that privilege
changes the mutation port and authorization, not the hash, receipt, backup, or rollback proof. The
same principle applies to static Copy creation and update. A privileged update also needs to restore
the previous Unix mode after its elevated write rather than silently accepting the privileged
writer's default mode.

## Decision

`CreateManagedFile`, `CreateManagedFileWithBackup`, and `UpdateManagedFile` bind a payload-free
`requires_admin` flag. When true, action permission derivation includes Administrator and matching
old or new App receipts require the same persisted privilege identity. The existing action kinds,
backup paths, rollback paths, hashes, modes, and safe-state assessments remain unchanged.

Planning admits administrator static Copy files under the same absent-destination,
backup-eligible unowned destination, and unchanged in-place update rules as unprivileged files. It
includes journal infrastructure and exact destination/backup/rollback effects before approval.

Execution acquires the host administrator lock before revalidating the journal, receipt, and path
state. Protected writes, moves, removals, and mode restoration use the privileged filesystem port.
The returned non-cloneable execution capability owns the administrator guard until the App
lifecycle has saved the matching receipt and journal commit has finished. A receipt-save or commit
failure drops the guard only after leaving the journal recoverable.

Explicit recovery uses the same safe-state matrices as unprivileged creation and update. Its Plan
includes Administrator only when the exact current state requires removing a protected created
file, restoring a protected backup or rollback file, or removing committed protected rollback
material. A matching durable receipt that needs only journal cleanup does not request elevation.

The Action IR and journal remain in the Unreleased development line. The new optional flags default
to false when decoding an earlier schema-v1 unprivileged journal, so the schema number does not
change.

## Consequences

- Static Copy create, backup-aware create, update, and uninstall now share one recoverable contract
  on user and administrator paths.
- The built-in Unix `app/docker-engine` lifecycle no longer has an unjournaled create/update window.
- Privileged update preserves the previously observed Unix mode through an explicit elevated mode
  operation.
- Receipt-only recovery and stale-journal cleanup never trigger administrator authorization.
- JSON merge follows the separate key-owned contract in
  [ADR 0057](0057-key-owned-json-merge-transactions.md); generators, relocation, stale-prune, Shell,
  and Sys action migration remain separate work.
