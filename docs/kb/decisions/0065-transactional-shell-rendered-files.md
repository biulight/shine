# 0065 — Shell rendered outputs use file-scoped transactions

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/{planner,shell,shell_action_executor}.rs`

## Context

Transformed Shell commands execute a Shine-owned file below `rendered/shell/`. Install and upgrade
previously rewrote that file before the launcher journal existed. A later receipt failure could
therefore leave new rendered bytes behind while launcher recovery projected the command back to its
previous receipt. Receipt equality is not a sufficient commit boundary because current environment
values affect rendered bytes but are intentionally absent from `shell-manifest.toml`.

Rendered files and embedded preset cache files do not share one ownership boundary. A rendered path
is file-scoped and may be consumed by multiple command receipts; embedded cache remains category
deployment material with force-sensitive extraction semantics.

## Decision

Action IR v1 adds `ReplaceShellRenderedFile` for approved install or upgrade when transformed output
is missing or differs by hash or mode. The action binds:

- optional previous and required desired file hash/mode identities without either payload;
- the canonical same-directory `.shine.rollback` path;
- every selected command receipt transition consuming that rendered path;
- an explicit per-action `receipt-committed` marker.

The Shell journal is durable before an existing exact file moves to rollback and the prepared output
is written. Rendered actions run before dependent launcher actions. After the desired receipt set is
saved, Core verifies the desired file and receipts, records the positive marker, then removes only
exact rollback material.

Before that marker, recovery first projects desired receipt transitions back to their previous
boundary, then removes an exact transaction-created file or restores the exact previous file. After
the marker, recovery keeps the exact desired file and may clean only exact rollback material. A
non-file destination, occupied or changed rollback, modified destination, or conflicting receipt
blocks all recovery mutation.

Embedded cache replacement, rendered uninstall, external snapshot uninstall, execution-time live
rendering, and profile sentinel blocks remain outside this action.

## Consequences

- Lifecycle rendering now participates in the same receipt-coherent recovery graph as snapshots and
  launchers, including environment-only output changes whose receipts compare equal.
- The security Plan observes rendered and rollback paths and grants write/remove access for both.
- Runtime live rendering remains atomic but is invocation-scoped and does not create a lifecycle
  operation journal.
- Embedded cache keeps its existing behavior until a separate category/source ownership proof is
  defined.
