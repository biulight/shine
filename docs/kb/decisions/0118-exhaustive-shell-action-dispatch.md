# ADR 0118: Exhaustive Shell Action dispatch

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Shell journal validation, receipt-marker eligibility, commit checks, rollback cleanup and recovery
dispatch independently matched the cross-domain Action IR. Wildcard fallbacks and independent
cleanup branches made support for a new Shell resource depend on synchronizing these lists manually.

## Decision

- Convert Action kinds exhaustively to a private borrowed `ShellAction` view. Explicitly reject
  every non-Shell variant with the existing unsupported-journal diagnostic. This conversion is
  structural; Action IR validation still runs first and retains all resource/rollback contracts.
- Use the same view for journal admission and receipt-marker eligibility. Launcher creation and
  update retain their matching-receipt boundary without a positive marker; all other supported
  Shell actions retain their explicit receipt-committed marker requirement.
- Exhaustively dispatch marker checks, commit preflight and cleanup. Check every action's live
  resource/receipt state before cleaning any rollback material. Preserve action order, diagnostic
  wording, short circuits, resource-specific missing-file tolerance and final journal removal.
- Use exhaustive Shell dispatch for recovery receipt projection and resource-handler selection.
  Keep the existing domain handlers, forward receipt projection, reverse resource recovery,
  observation and permission order, lifecycle/privileged locks and fresh approval checks.

## Consequences

New Action kinds require an explicit domain classification; new Shell variants require decisions
in each exhaustive phase. The view borrows existing DTO fields and introduces no serialized state,
cached observations or execution authority. Commands, Plans/fingerprints, receipts, journal schemas
and supported recovery behavior remain unchanged.

Regressions exercise mixed cache/rendered/launcher commit for both launcher creation and update:
marker eligibility, late cleanup failure, retry after partial cleanup, changed resources blocking
all cleanup, and valid foreign-domain Action IR rejected by marker/commit/recovery-review entry
points without resource mutation or journal replacement. Existing lifecycle and recovery tests
continue to cover snapshot, profile, removal and interrupted execution paths.
