# 0060 — Receipt-owned Shell launcher removal requires a durable commit marker

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/shell_action_executor.rs`,
  `core/src/runtime/{launcher,shell,planner}.rs`
- **Update**: [ADR 0064](0064-transactional-external-shell-snapshots.md) reuses positive commit
  evidence for raw external snapshot replacement; snapshot uninstall remains separate.

## Context

Uninstall previously removed a managed launcher before saving the command manifest. A crash in that
window could leave the old receipt pointing at a missing Unix launcher or an incomplete Windows shim
pair. Removing the receipt first has the inverse ambiguity: receipt absence alone cannot distinguish
a committed uninstall from a manifest save that succeeded immediately before the process crashed.

Launcher bytes must not enter Action IR or the journal. Foreign or user-modified launchers also lack
the exact prior-state proof required for automatic removal.

## Decision

Action IR v1 adds `RemoveShellLauncher`. It binds the complete previous command receipt and every
platform launcher resource deterministically reconstructed from that receipt, plus each canonical
same-directory `<name>.shine.rollback` path. A resource records only its symlink target or file
hash/mode identity.

The action is eligible only during approved uninstall when the exact old receipt still exists,
every Unix launcher or Windows shim resource matches it, and every rollback path is absent. The
Shell Plan observes and grants removal of each destination plus write/remove access to every
rollback path. An occupied rollback blocks before mutation. Foreign and modified launchers remain
preserved and do not inherit transactional ownership.

Core writes `shell-operation-journal.toml`, moves every launcher resource to rollback, saves the
manifest without the command receipt, and then durably records the action's `receipt-committed`
state in the journal. Only after that positive marker may commit remove exact rollback material and
clear the journal. The operation lock spans apply, manifest save, marker persistence, and cleanup.

Explicit `shine shell recover` treats the marker as the commit boundary. Before the marker, an exact
old receipt authorizes restoring exact rollback resources. If the receipt is absent but the marker
is not present, recovery first reconstructs the complete old receipt and then restores those
resources. After the marker, recovery preserves the completed uninstall and removes only exact
rollback material. A conflicting receipt or any changed destination or rollback resource blocks and
preserves the journal.

## Consequences

- Interrupted launcher removal cannot silently turn a manifest-write crash into a committed
  uninstall.
- Unix launchers and Windows shim pairs share one payload-free removal and recovery contract.
- Modified launchers remain user-owned and are reported as conflicts; their existing uninstall
  receipt behavior is unchanged.
- Shared snapshot/render resources and sentinel profile blocks remain separate Phase 4D actions.
