# ADR 0104: Typed planning semantics and domain phases

- **Date**: 2026-10-09
- **Status**: Accepted

## Context

App code boundaries inferred from resource prefixes confused static data with generators, hooks
and artifacts. Validation codes inferred from English error chains changed with category names.
App relocation eligibility and approval gates were duplicated, while large planning/recovery
functions and implementation-text boundary tests made safe extraction difficult.

## Decision

- Record App code entry kinds at actual assessment sites; assemble boundaries from those records.
  Keep trust evaluation and triggered-code permissions unchanged.
- Carry private typed metadata errors from missing-reference, duplicate-target/command and Bun
  policy checks to the validation projection. Unknown metadata errors remain `invalid_metadata`.
- Add optional `PlanStepV1.kind` for semantic intent used by relocation, stale pruning, forced
  removal and shared Shell delivery. Diagnostic codes remain output. Schema 2 is retained because
  the field is additive, defaults to absent and is omitted when absent; old step JSON still reads.
  The full serialized Plan fingerprint binds present kinds, so changing/removing one invalidates
  approval. Missing intent cannot authorize a transition that requires it.
- Share static App relocation eligibility between Plan and Action IR construction. Share exact
  App approval/IR/permission validation before action-specific checks. Do not introduce a generic
  cross-domain transaction engine or execute semantic Plan steps directly.
- Keep `planner.rs` and `action_executor.rs` as stable facades. Extract domain planning phases,
  per-resource App convergence and typed recovery assessment/apply handlers. Recovery keeps
  receipt-conflict checks, reverse execution order, live identity checks and positive commit markers.
- Verify planner boundaries through observation-only generic callers, a compile-fail mutation
  example and Host operation assertions, rather than fixed source-file separators or signatures.

## Consequences

Static App resource names no longer alter code consent; validation identities survive wording
changes. Existing public entry points, diagnostic spellings, Action IR and journal schemas remain.
Some newly typed Plans have different fingerprints and must be reviewed again. Domain tests move
with their modules and continue to cover ownership, user edits, privilege and interrupted recovery.
