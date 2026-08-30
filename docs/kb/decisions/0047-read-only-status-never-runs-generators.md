# 0047 — App status runs generators only through explicit opt-in

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/runtime/app.rs`, `core/src/runtime/planner.rs`

## Context

Automatic App generators historically ran while computing update/status output. That makes a
read-oriented command execute Preset code and possibly access the network, and it prevents future
frontends from treating inspection as an observation-only operation. A security Plan already
models generator execution conservatively without running it.

## Decision

App list, ordinary info/status/update, and security planning never execute a generator. When a
dynamic result cannot be known without execution, inspection reports
`app_generator_not_evaluated`, displays an actionable warning, and does not claim the resource is
current.

`shine app info`, top-level `shine info`, targeted `shine update`, and global `shine update` accept
`--run-generators`. The flag is an explicit per-invocation authorization to execute every selected
App generator, including `auto = false` generators, in order to calculate transformed desired
content, hashes, status, and optional diffs. It never writes destinations or manifests and never
runs hooks or artifacts. Global update selects installed App categories only and executes each
generator at most once.

External and overlay generators still require a matching target-local trust grant. Evaluation
continues across failures, reports `app_generator_evaluation_failed` or
`app_generator_trust_required` per file, and returns nonzero after reporting the remaining results.

Only `--run-generators`, explicit `app refresh`, and an approved lifecycle mutation may execute a
generator. Mutating operations retain exact Plan approval and ownership checks; evaluation retains
scoped external-code trust, output limits, environment allowlists, and no-write semantics.

## Consequences

- Default read-oriented commands remain local observation paths with no Preset process or network
  effects.
- Developers can explicitly evaluate dynamic desired content before installation or upgrade without
  adding a separate preview command.
- Structured status distinguishes not-evaluated, evaluation-failed, and trust-required results
  rather than overloading `current`.
- Callers must opt in deliberately; merely requesting info, status, or a security Plan never runs
  code.
