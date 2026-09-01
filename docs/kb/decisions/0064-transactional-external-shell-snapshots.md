# 0064 — Raw external Shell snapshots use category-scoped transactions

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/{planner,shell,shell_action_executor}.rs`

## Context

External Shell presets in snapshot mode materialize a whole category under
`<shine_dir>/installed/shell/<category>`. The category tree is shared by every command in that
category, but command ownership is recorded as separate entries in `shell-manifest.toml`.
Previously the tree was replaced before the launcher journal existed, using random stage and backup
paths that were deleted before command receipts committed. A crash could therefore leave new shared
source bytes with old command receipts and no explicit recovery path.

The receipt transition alone is not sufficient commit evidence. A category auxiliary file may
change without changing any selected command receipt, and a manifest save can become durable just
before a crash. Transformed commands also have a distinct rendered-output boundary; admitting them
to a snapshot-only proof would leave the rendered file outside the transaction.

## Decision

Action IR v1 adds `ReplaceShellSnapshot` for approved install or upgrade of an external snapshot
selection whose selected commands require no transforms. The action target is the category, not an
individual command. It binds:

- the exact sorted previous and desired regular-file tree identities without file payloads;
- deterministic sibling `.shine.stage` and `.shine.rollback` directories;
- every selected command's optional previous and required desired receipt;
- an explicit per-action `receipt-committed` journal marker.

The Shell journal is durable before staging. Staging may contain only an exact subset of desired
files; any extra or changed file blocks recovery. The old category tree moves to rollback before the
stage becomes active. The complete selected receipt set is then saved atomically and the positive
commit marker is persisted before exact rollback cleanup.

Before that marker, recovery evaluates command-launcher actions against a virtual manifest with the
snapshot receipt transitions reversed. Apply writes that previous manifest state first, then rolls
back launcher and snapshot actions. This prevents a newly written receipt from incorrectly
preserving a launcher when the shared snapshot transaction is still uncommitted. After the marker,
recovery preserves the exact desired tree and removes only the exact previous rollback tree. Any
changed destination, stage, rollback, or conflicting receipt blocks all recovery mutation.

Embedded preset cache, transformed rendered outputs, snapshot uninstall, and profile sentinel
blocks remain outside this action until separate ownership proofs are defined.

## Consequences

- Raw external snapshot creation and replacement now have journal-before-mutation ordering and an
  explicit recovery Plan across the category tree, command receipts, and launcher actions.
- Random snapshot backup paths are no longer used for eligible operations; occupied deterministic
  stage or rollback paths block before mutation.
- Category auxiliary-file changes can commit safely even when command receipt values are unchanged.
- Render/cache/profile work remains visible in the Phase 4 inventory rather than inheriting this
  narrower proof.
