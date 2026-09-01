# 0066 — Embedded Shell cache uses category-scoped file-patch transactions

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/{planner,shell,shell_action_executor}.rs`

## Context

Embedded Shell install materializes every asset in the selected category below
`presets/shell/<category>/`. Missing files are created normally; existing files are overwritten by
upgrade or explicit `--replace-managed`, and unrelated files are preserved. This deployment previously ran before
the Shell operation journal. A later launcher or manifest failure could therefore leave created or
replaced cache bytes active while recovery returned command receipts and launchers to their previous
boundary.

Whole-tree replacement would not preserve the established merge semantics: the cache can contain
untouched local files, and a command target still deploys category-scoped sibling material.

## Decision

Action IR v1 adds one `ReplaceShellCache` action per selected category that has actual cache writes.
The action contains one entry per changed file and binds:

- an optional previous and required desired hash/mode identity without either payload;
- a canonical same-directory `.shine.rollback` path for every changed file;
- all selected command receipt transitions for the category;
- an explicit per-action `receipt-committed` marker.

The Plan observes every changed destination and rollback path. Non-file destinations, occupied
rollback paths, and destination/rollback aliasing block before mutation. The journal is durable
before existing files move to rollback and new bytes are written. Cache actions execute before
rendered-file and launcher actions.

Before the marker, recovery first projects desired receipts back to their previous boundary, then
reverses each exact file state in reverse order: remove an unchanged created file, restore an exact
moved or replaced file, or leave an unstarted file alone. After the marker, recovery preserves every
exact desired file and removes only exact rollback material. Any modified destination, rollback, or
conflicting receipt blocks the whole cache action.

## Consequences

- First install and `--replace-managed` cache writes share the Shell receipt-coherent recovery graph.
- Existing files skipped by install without `--replace-managed` and unrelated category files remain
  untouched.
- The Action IR and journal contain only identities; prepared embedded bytes stay execution-local.
- Embedded cache uninstall, rendered uninstall, execution-time live rendering, snapshot uninstall,
  and profile sentinel blocks remain outside this action.
