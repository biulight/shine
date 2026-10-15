# ADR 0111: Lifecycle phases and CLI dispatch boundaries

- **Date**: 2026-10-15
- **Status**: Accepted

## Context

Shell resource preparation and recovery accumulated large multi-resource functions. App upgrade
mixed preparation, installed-file convergence, stale removal and additions. CLI review mixed
approval ownership with rendering, while binary dispatch and source-text tests coupled changes
to file layout. Broad result-match fallbacks hid new App action variants from the compiler.

## Decision

- Preserve every existing App result mapping with explicit exhaustive enum matches.
- Keep process setup in the binary; move command dispatch and parser tests into the CLI library.
  Keep commands that bypass runtime initialization in a separate pre-config dispatcher. Preserve
  early completion, directed env writes, exit codes and notification ordering.
- Separate pure Plan rendering from approval orchestration. Keep existing display classifiers and
  unknown-diagnostic fallbacks: this structural change does not add Plan fields or change fingerprints.
- Split App upgrade into preparation, installed-file convergence, stale pruning, additions and
  final receipt/hook handling. Keep the same locked operation, assessment order and save points.
- Split Shell resource preparation, apply, receipt commit, recovery review and recovery apply.
  Resource handlers remain domain-specific. The orchestrators retain journal publication, lock
  lifetimes, reverse rollback order, per-action durability and final journal removal.
- Replace the CLI authority test's fixed source-file/call-string assertions with process-level
  consent coverage and frontend prevalidation/apply race coverage. Retain dependency checks,
  observation-only capability tests and existing source checks for forbidden legacy fallbacks.

## Consequences

Public commands, output, serialized Plans, receipts and journal schemas are unchanged. New App
result variants require explicit CLI decisions. Module moves no longer require a specific approval
method spelling in a particular file. Approval revalidation, positive commit evidence and ownership
checks remain mandatory; no cross-domain transaction engine is introduced.
