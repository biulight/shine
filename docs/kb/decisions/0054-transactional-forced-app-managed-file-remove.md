# 0054 — Forced App removal stages the user-modified file

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

[ADR 0052](0052-transactional-app-managed-file-remove.md) and
[ADR 0053](0053-transactional-app-backup-restoring-remove.md) cover ordinary removal only while the
managed destination still matches its receipt. `app uninstall --force` deliberately overrides that
ownership guard for a user-modified destination. The legacy executor deleted those modified bytes
directly, so an interruption around receipt removal could not reconstruct the pre-uninstall state.

Storing the modified file in the journal would expose user data. Reusing the ordinary removal
action would also erase an important review distinction: its receipt hash identifies the previous
managed content, while rollback safety must bind the different current user-modified content.

## Decision

Action IR v1 adds the distinct `ForceRemoveManagedFile` action for a receipt-owned, unprivileged,
static Copy whose current regular-file hash differs from its receipt hash. It binds both hashes,
the current file mode, the canonical same-directory `.shine.rollback` path, and the optional
canonical persistent backup's mode and hash. It stores no file bytes. An unchanged destination
still uses the ordinary removal action even when the request includes `--force`.

The approved Plan must contain the `app_user_modification_override` diagnostic and every derived
path permission. Under the host operation lock, execution revalidates the exact receipt, modified
destination, optional backup, and absent unclaimed rollback path. It writes the prepared journal,
moves the modified destination to `.shine.rollback`, optionally moves `.shine.bak` to the
destination, and records the action as applied.

Before receipt commit, recovery preserves the user's pre-uninstall state. Without a persistent
backup it moves the exact modified rollback file back to the destination. With a backup it reverses
either one or both completed renames, restoring the exact modified destination and the exact
persistent backup. If receipt absence is visible without the positive journal marker, recovery
first reconstructs the exact previous receipt. After `receipt-committed`, recovery keeps the
completed uninstall state and removes only rollback material whose mode and hash still match the
captured user-modified file.

Any changed kind, mode, hash, receipt, destination, backup, or rollback path blocks and preserves
all state. At adoption, administrator paths, JSON merge, generators, relocation, and stale-prune
remain outside this action and keep their existing executors; [ADR 0055](0055-privileged-app-removal-reuses-typed-transaction.md)
later adds privileged execution without changing these safe states.

## Consequences

- Forced static Copy removal is still explicitly destructive, but interruption no longer turns its
  receipt boundary into an unrecoverable deletion window.
- The journal remains payload-free while the same-directory rollback file temporarily contains
  user-modified data and must be treated as sensitive.
- Review and lifecycle results retain an explicit forced-removal signal rather than presenting the
  operation as an ordinary owned-file removal.
- Administrator removal needs separate execution/authorization semantics; ADR 0055 supplies them
  while reusing this transaction proof.
