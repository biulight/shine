# 0053 — App backup-restoring removal moves both owned paths

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

[ADR 0052](0052-transactional-app-managed-file-remove.md) journals ordinary removal only when an
App receipt has no persistent backup. An App installed over a pre-existing user file instead owns
two related paths: the managed destination and the fixed `.shine.bak` containing the user's
original bytes. Legacy uninstall removes the managed file and restores that backup, but an
interruption between either rename, receipt removal, and transaction cleanup could leave ownership
ambiguous.

Serializing either file into the journal would expand its plaintext surface. Treating receipt
absence alone as commit would also risk rolling a completed uninstall back over the restored user
file.

## Decision

Action IR v1 adds `RemoveManagedFileWithBackup` for one unchanged, receipt-owned, unprivileged
static Copy whose fixed persistent backup is an unchanged regular file. It binds the destination,
canonical persistent backup and transaction rollback paths, the mode and hash of both files, and
the previous payload-free receipt fields. It stores neither file's bytes.

Under the host operation lock, execution revalidates the approved Plan, exact receipt, both regular
files, their modes and hashes, and an absent unclaimed rollback path. It writes a prepared journal,
renames the managed destination to `.shine.rollback`, renames `.shine.bak` to the destination, and
then records the action as applied. App lifecycle removes and saves the receipt before commit
durably records `receipt-committed`. Commit keeps the restored user destination and removes only
unchanged managed rollback material.

Before receipt commit, recovery recognizes exactly three safe states:

| Destination | Persistent backup | Transaction rollback | Recovery |
|---|---|---|---|
| exact managed | exact user original | missing | clear journal; mutation did not start |
| missing | exact user original | exact managed | move rollback back to destination |
| exact user original | missing | exact managed | move destination back to backup, then rollback back to destination |

If the old receipt is absent without the positive marker, recovery first reconstructs that exact
receipt and then restores the corresponding safe state. After `receipt-committed`, the only valid
state is the exact user original at the destination, a missing persistent backup, and either exact
managed rollback material or an already missing rollback; recovery removes only the former.

Any other kind, mode, hash, receipt, or path combination blocks and preserves all state.
[ADR 0054](0054-transactional-forced-app-managed-file-remove.md) gives force a distinct action;
[ADR 0055](0055-privileged-app-removal-reuses-typed-transaction.md) adds privileged execution.
JSON merge, relocation, stale-prune, and upgrade-internal removal remain outside this action.

## Consequences

- A normal uninstall can restore a pre-install user file without exposing it or the managed file
  in the journal.
- An interruption between the two renames is reversible because both files still have bound,
  distinct identities.
- Receipt absence remains insufficient proof of completion; only the durable journal marker lets
  recovery keep the restored user file and discard unchanged managed rollback material.
- The destination temporarily changes ownership during the transaction, so recovery binds and
  checks both modes and hashes before moving either path.
