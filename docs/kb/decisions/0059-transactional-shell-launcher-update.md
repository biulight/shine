# 0059 — Receipt-owned Shell launcher updates use same-directory rollback material

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/shell_action_executor.rs`,
  `core/src/runtime/{launcher,shell,planner}.rs`

## Context

ADR 0058 journals first-time launcher creation, but an installed command can later require a
different launcher because its source path, runtime, environment wrapper, dependency policy, live
render target, or platform shim content changed. Replacing the launcher before the new command
receipt is durable can leave the old receipt pointing at new resources. Creation rollback cannot
restore the old symlink or generated files, and serializing launcher bytes into Action IR would
violate the payload-free action contract.

A logical Windows launcher expands to a PowerShell file and a sibling cmd shim. Planning only the
primary path would approve fewer resources than execution mutates. Foreign or already modified
launchers also lack the exact prior-state proof needed for automatic rollback.

## Decision

Action IR v1 adds `UpdateShellLauncher`. The action binds complete previous and desired
command-receipt contracts plus one entry for every changed platform launcher resource. Each entry
records the previous and desired symlink target or file hash/mode and the canonical same-directory
`<name>.shine.rollback` path; it contains no launcher bytes.

The action is eligible only during an approved install or upgrade when the old receipt still owns
the command, every current launcher resource exactly matches the deterministic launcher reconstructed
from that receipt, the old and desired resource shapes agree, at least one resource changes, and
all rollback paths are absent. Modified, foreign, unsupported legacy, removal, shared
snapshot/render, and profile paths retain their existing lifecycle behavior.

The Shell planner and executor both expand the launcher through the same platform helper. Plans
observe and grant every resource, including both Windows files, plus write/remove access to each
changed resource's rollback path. Core writes `shell-operation-journal.toml` before mutation, moves
each changed old resource to rollback, writes its replacement, persists the new command receipt,
verifies every desired and rollback identity, removes unchanged rollback material, and finally
clears the journal. The operation lock remains held through receipt commit and cleanup.

Explicit `shine shell recover` distinguishes the two receipt boundaries. While the exact old
receipt remains, it accepts only the not-started, moved-old, or replaced-with-exact-rollback states
and restores the old resource. Once the exact desired receipt is durable, it preserves the
replacement and removes only an exact old rollback resource. Any changed resource, changed rollback,
missing/conflicting receipt, or unrecognized state blocks recovery and preserves the journal.

## Consequences

- Interrupted receipt-owned launcher updates are reversible without storing launcher bytes.
- Install and upgrade share one receipt-gated transaction and the same explicit recovery command.
- Windows Plan approval now binds both shim files for create, update, and removal paths.
- A launcher that no longer matches the deterministic old receipt is deliberately outside this
  action; `--replace-managed` does not convert user-modified bytes into transaction-owned rollback.
- Launcher removal is specified separately by ADR 0060; shared snapshot/render resources and
  sentinel profile blocks remain separate Phase 4D actions.
