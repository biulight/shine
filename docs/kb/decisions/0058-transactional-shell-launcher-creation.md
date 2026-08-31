# 0058 — First-time Shell launcher creation is a receipt-gated transaction

- **Status**: Accepted
- **Date**: 2026-08-31
- **Evidence**: `core/src/action.rs`, `core/src/runtime/shell_action_executor.rs`,
  `core/src/runtime/{launcher,shell,planner}.rs`, `cli/src/shells/recovery.rs`

## Context

A Shell command becomes active through a launcher and a command-scoped `shell-manifest.toml`
receipt. The launcher can be a Unix symlink, a generated Unix Bun/live script, or a Windows
PowerShell/cmd shim pair. Previously these resources were created before the receipt without a
journal, so interruption could leave an unowned launcher that a later install treated as existing
state.

Shell snapshots and rendered files are shared deployment material, while shell configuration is a
user-owned file containing only a Shine sentinel block. Whole-file App rollback cannot safely own
either boundary. Existing managed launcher replacement and removal also require prior-resource
rollback proofs rather than creation rollback.

## Decision

Action IR v1 adds `CreateShellLauncher`. One action binds the exact command-scoped receipt and every
platform resource: symlink destination/target or generated-file destination/content hash/mode. It
contains no launcher bytes. The action is eligible only during install when the command has no
receipt and every launcher resource is absent; existing, stale, foreign, upgrade, and uninstall
paths do not inherit the creation rollback proof. Receipt-owned update is defined separately by
ADR 0059; the other paths retain their previous executor until narrower actions define rollback.

Core writes `shell-operation-journal.toml` before the first launcher mutation while holding the
cross-process operation lock. Unix native creation applies one symlink, Unix Bun/live creation one
marked executable file, and Windows creation two marked shim files under one action. The journal
records apply progress after all resources for the action are written, but recovery always observes
every resource, including a partially written prepared action.

`shell-manifest.toml` is then persisted and must match the action's complete receipt contract before
Core removes the journal. Profile editing happens only after that commit. An interrupted journal
blocks ordinary Shell lifecycle Plans and is resolved only through a freshly reviewed
`shell-recovery` Plan exposed as `shine shell recover [--yes]`.

Without a matching receipt, recovery removes only resources whose type, target or content hash, and
mode still match the action; missing resources are no-ops and any changed resource blocks all
recovery mutation. With the exact receipt already durable, recovery preserves the manifest-owned
launcher and removes only the stale journal. A conflicting receipt blocks.

## Consequences

- First-time launcher creation now has journal-before-mutation and receipt-before-commit ordering on
  Unix and Windows, including interruption between the two Windows shim writes.
- User-created or user-modified launcher paths are never removed by recovery.
- Snapshot/render cache may remain as harmless Shine-owned material after rollback; profile files
  are not touched by launcher recovery.
- Launcher removal, shared snapshot/render resources, and sentinel profile blocks remain Phase 4D
  follow-up actions with distinct ownership proofs; ADR 0059 defines receipt-owned update.
