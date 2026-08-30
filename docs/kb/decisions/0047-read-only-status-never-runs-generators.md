# 0047 — Read-only App status never runs generators

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/runtime/app.rs`, `core/src/runtime/planner.rs`

## Context

Automatic App generators historically ran while computing update/status output. That makes a
read-oriented command execute Preset code and possibly access the network, and it prevents future
frontends from treating inspection as an observation-only operation. A security Plan already
models generator execution conservatively without running it.

## Decision

App list, info, status, update, and security planning never execute a generator. Static generator
input changes may report that refresh is available. When dynamic output cannot be known without
execution, inspection reports a stable refresh-required diagnostic and does not claim the generated
resource is current.

Only explicit `app refresh` and an approved lifecycle mutation may execute an automatic generator.
Execution remains subject to scoped external-code trust, exact Plan approval, output limits,
environment allowlists, ownership checks, and last-known-good preservation.

## Consequences

- Read-oriented commands are local observation paths with no Preset process or network effects.
- Dynamic generated content requires an explicit mutation to discover and apply upstream changes.
- Structured status gains a refresh-required reason rather than overloading `current`.
- Setup orchestration and other future frontends can consume inspection without executing code.
