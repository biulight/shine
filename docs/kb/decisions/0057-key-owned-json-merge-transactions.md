# 0057 — App JSON merge recovery owns keys, not the whole destination

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`

## Context

App `json-merge` owns declared top-level keys inside a user-owned JSON object. The legacy lifecycle
correctly preserved other keys during ordinary install and uninstall, but each mutation rewrote the
whole file without a journal. Reusing static Copy rollback would be unsafe: after an interruption,
restoring the complete previous file could overwrite unrelated settings changed by the user or the
application.

The previous and desired JSON values may also contain expanded configuration. They must not be
serialized into Action IR or the operation journal.

## Decision

Action IR v1 adds `MergeManagedJson` and `RemoveManagedJson`. Both bind the destination, canonical
same-directory `.shine.rollback` path, declared unique top-level keys, whole-file pre-operation
hash/mode, and managed-subset hashes needed for receipt checks. Update binds the previous receipt
hash; forced removal binds the receipt and current managed-subset hashes separately. Neither action
contains prior or desired JSON values.

When a destination exists, execution writes the prepared journal and renames the exact whole file
to rollback material before writing the merged or pruned JSON object. The rollback file is an
ephemeral value source and whole-file identity, not a claim that Shine owns every key. A newly
created destination has no rollback file.

Before receipt commit, recovery accepts only these semantic states:

| Destination managed keys | Exact rollback | Recovery |
|---|---|---|
| previous values | missing | mutation did not start |
| missing destination | previous file | rename the previous file back |
| desired/absent values | previous file | restore only previous managed keys into current JSON |
| previous values | previous file | key restoration already completed; remove rollback |

For creation at an absent destination, recovery removes the file only when it contains no unrelated
keys. If unrelated keys now exist, it removes only the managed keys. Any changed managed value,
invalid/non-object JSON, changed rollback kind/hash/mode, or receipt conflict blocks recovery and
preserves all state.

Uninstall treats receipt absence without `receipt-committed` as uncommitted: it reconstructs the
old payload-free JSON receipt and restores only the managed keys. After `receipt-committed`, current
JSON is user-owned; recovery preserves it even if the user has reintroduced a formerly managed key
and removes only exact rollback material. Install/update receipt commit similarly preserves the new
managed state and removes only exact rollback material.

## Consequences

- JSON install, in-place update, forced removal, receipt commit, and explicit recovery use the
  Phase 4 journal executor.
- Unrelated JSON values changed after interruption survive rollback. Formatting may be normalized
  when key restoration rewrites a valid JSON object, but unrelated values are not replaced.
- The same-directory rollback file temporarily contains the complete pre-operation JSON object and
  must be treated as sensitive transaction material.
- JSON relocation is specified separately by
  [ADR 0063](0063-transactional-app-json-relocation.md); generators, remaining Shell/Sys actions,
  and opaque execution remain outside this action slice.
