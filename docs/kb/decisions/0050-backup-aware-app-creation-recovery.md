# 0050 — Backup-aware App creation restores only unchanged transaction state

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

ADR 0048 journals creation only when an App Copy destination is absent. Existing App install also
supports an unowned destination by moving it to the fixed `<name>.shine.bak` path before writing the
managed file. An interruption can therefore leave the original at either destination or backup,
with or without the new managed bytes, before the App receipt becomes durable.

Persisting original file bytes in the journal would enlarge a long-lived plaintext surface and is
unnecessary because the backup is already the product's ownership-preserving artifact. Recovery
must nevertheless distinguish a transaction-created backup from a pre-existing backup and must not
overwrite either path after the user changes it.

## Decision

Action IR v1 adds a typed backup-aware managed-file creation action for an unprivileged static Copy
whose destination is an unowned regular file and whose fixed backup path is absent. The action binds
the resolved destination and backup paths plus hashes of the original and desired bytes. It carries
neither byte payload. The approved install Plan observes both path kinds and bytes and includes
destination write/remove, backup write, journal write/remove, and receipt write capabilities before
execution.

The Action IR and App journal are still in the Unreleased development line, so this completes their
schema-v1 creation vocabulary before its first release rather than introducing schema v2.

Execution revalidates the original destination hash and absent backup under the host-provided
cross-process operation lock. It then persists the prepared journal, renames destination to backup,
writes the desired bytes atomically, persists the applied action state, saves a receipt containing
the exact backup path, and commits only after re-reading that receipt. A pre-existing backup blocks
planning and execution instead of being replaced.

Without a matching durable receipt, recovery accepts only these states:

| Destination | Backup | Recovery |
|---|---|---|
| original hash | missing | clear the journal; mutation did not start |
| missing | original hash | rename backup back to destination |
| desired hash | original hash | remove desired bytes, then rename backup back |

Every other combination, including a non-regular path even with matching readable bytes, is blocked
and preserves destination, backup, and journal. The recovery Plan binds both current observations
and requests every remove/write capability before mutation.
A matching receipt means ownership committed: recovery preserves both managed destination and
backup and removes only the stale journal.

This slice does not cover managed update, uninstall, JSON merge, generators, administrator paths,
or opaque execution.

## Consequences

- Interrupted backup creation is recoverable without serializing original or managed bytes.
- The fixed `.shine.bak` artifact becomes transaction evidence and is never overwritten by this
  action when already present.
- Recovery remains rollback-only, explicit, freshly approved, and blocked by any post-interruption
  change to either path.
- Managed update and uninstall still need separate rollback-material and action contracts before
  joining the journal executor.
