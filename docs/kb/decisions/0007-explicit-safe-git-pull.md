# 0007 — Git-managed presets use explicit, fast-forward-only pulls

- **Status**: accepted
- **Evidence**: `cli/src/git_pull.rs`, `shine pull`, `update --pull`, `upgrade --pull`

## Context

External presets and overlays are often Git checkouts, but updating them manually adds friction.
Making ordinary `shine update` mutate those repositories would violate its read-only status-check
semantics and could unexpectedly interact with local edits or divergent branches.

## Decision

Git synchronization is explicit through `shine pull` or a `--pull` option. Pull resolves the
effective preset sources, refuses dirty worktrees, validates tracking branches, de-duplicates Git
roots, and uses `git pull --ff-only`. Combined commands pull first and reload configuration before
checking or applying presets. Shine never stashes, rebases, resets, or resolves conflicts.

## Consequences

- Plain `shine update` remains read-only.
- Pull failures stop combined commands before status checks or installed-file changes.
- Users resolve local changes, divergence, authentication, and upstream configuration with Git.
