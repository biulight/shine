# 0051 — App managed-file updates use same-directory transaction rollback material

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

The first Phase 4 actions can recover creation at an absent destination or creation after moving an
unowned file to its persistent `.shine.bak`. Updating a manifest-owned file has a different failure
window: overwriting the destination before the replacement receipt is durable destroys the only
copy of the previous managed bytes, while storing those bytes in the Action IR or journal would
create a long-lived plaintext payload that may contain expanded environment or secret values.

The existing App installer also preserves the mode of an in-place Unix file overwrite. Moving the
old file aside and creating a new destination must not silently weaken or broaden its permissions.

## Decision

Action IR v1 adds `UpdateManagedFile` for one in-place, unprivileged, static Copy replacement. The
action binds the destination, the canonical same-directory `<name>.shine.rollback` path, the prior
persistent `.shine.bak` identity if any, the prior Unix mode when available, and original/desired
content hashes. It contains neither byte payload.

The planner admits this action only when the existing App receipt owns the same destination, the
live regular file still matches that receipt, the desired bytes differ, and the rollback path is
absent and unclaimed. Generator, JSON merge, relocation, forced, administrator and stale-removal
paths remain outside this slice. Both install of an already managed target and App upgrade use the
same action executor when these conditions hold.

Execution revalidates the exact approval, previous receipt, destination kind/hash/mode and absent
rollback path while holding the host operation lock. It then:

1. writes the prepared journal;
2. renames the previous managed destination to the same-directory rollback path;
3. atomically writes the desired bytes and restores the prior mode;
4. writes the applied journal state;
5. returns to App lifecycle orchestration, which persists the replacement receipt;
6. re-reads that receipt, removes unchanged rollback material, then removes the journal.

Renaming rather than copying avoids creating a second plaintext copy. The rollback path is still
sensitive transaction state: it is never overwritten, is removed promptly after commit, and is
preserved if its kind or hash changes.

Without a replacement receipt, recovery accepts only these states while the previous receipt is
still exact:

| Destination | Rollback | Recovery |
|---|---|---|
| original hash | missing | clear the journal; mutation did not start |
| missing | original hash | rename rollback back to destination |
| desired hash | original hash | remove desired bytes, then restore the previous destination |

With the replacement receipt durable, a missing rollback permits journal cleanup and an unchanged
original rollback is removed before journal cleanup. Every other destination, rollback or receipt
combination blocks recovery and preserves all paths. Recovery remains an explicit, freshly approved
`app-recovery` operation.

## Consequences

- Static in-place App updates no longer have an unjournaled overwrite window.
- The previous managed bytes may temporarily exist at a predictable sibling path, but are moved
  rather than copied and are guarded by exact receipt, mode, kind and hash checks.
- A pre-existing `.shine.rollback` blocks the Plan instead of being overwritten.
- Managed uninstall can reuse the transaction-material contract in a later slice, but persistent
  backup restoration, JSON merge and administrator updates still require separate action semantics.
- The Action IR and journal remain in the Unreleased line, so the update vocabulary completes
  schema v1 before its first release.
