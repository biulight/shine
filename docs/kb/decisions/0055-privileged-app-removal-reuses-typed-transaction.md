# 0055 — Privileged App removal reuses the typed transaction

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`, `core/src/runtime/host.rs`,
  `cli/src/apps/recovery.rs`

## Context

ADRs [0052](0052-transactional-app-managed-file-remove.md),
[0053](0053-transactional-app-backup-restoring-remove.md), and
[0054](0054-transactional-forced-app-managed-file-remove.md) define the safe states for ordinary,
backup-restoring, and forced static Copy removal, but initially limit execution to unprivileged
paths. The legacy privileged executor holds the cross-process administrator lock and uses elevated
filesystem primitives, yet deletes the destination before receipt commit and therefore lacks the
journaled recovery boundary.

Privilege changes how a path mutation is authorized and performed, not the file/receipt state that
makes rollback safe. Creating parallel administrator-only action kinds would duplicate the same
hash, mode, backup, receipt, and recovery state machines and risk semantic drift.

## Decision

The three managed-file removal actions gain a payload-free `requires_admin` flag. When set, action
permission derivation includes `Administrator`, and exact previous-receipt matching requires the
manifest's persisted `requires_admin` value. The flag does not change rollback classification or
safe-state assessment.

Planning admits receipt-owned administrator static Copy files under the same ordinary, backup, and
forced eligibility rules. It includes administrator permission only when the exact Plan will mutate
a protected destination, backup, or rollback path; a preserved user-modified file or receipt-only
recovery does not request elevation.

Execution holds the host-provided cross-process privileged-operation lock across journal checks,
receipt and path revalidation, the complete move sequence, journal state changes, receipt commit,
and rollback cleanup. Protected destination, backup, and `.shine.rollback` mutations use
`move_privileged` or `remove_privileged`; the user-owned journal and App manifest retain their
normal atomic persistence path while under the same lock. The executor returns a non-cloneable
execution capability that owns the lock guard for a privileged removal, and journal commit accepts
that capability instead of reacquiring the lock after the lifecycle saves the manifest. This keeps
the receipt write inside the same serialized transaction while still preserving the journal if the
write or commit fails.

Explicit recovery derives `Administrator` only when its current safe state requires a protected
path move or removal. The CLI reviews and approves that recovery Plan first, then obtains
administrator authorization before applying it. Receipt reconstruction or stale-journal cleanup
alone does not trigger administrator authorization.

## Consequences

- Ordinary, backup-restoring, and forced administrator static Copy removals now have the same
  interruption guarantees as their unprivileged equivalents.
- The manifest remains authoritative for privilege routing; active Preset metadata cannot silently
  downgrade a privileged receipt.
- Recovery clients outside the CLI must honor the Plan's `Administrator` permission before using a
  real privileged host.
- Privileged install/update follow the same principle through
  [ADR 0056](0056-privileged-app-create-update-reuse-typed-transactions.md). JSON merge, generators,
  relocation, stale-prune, Shell, and Sys action migration remain separate work.
