# ADR 0117: Pair Shell preparation and type App content assessment

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Shell transaction preparation maintained separate ordered Action and executable-payload vectors,
then zipped them during apply. Their positional correspondence was an implicit invariant.
App assessment represented ready content, unevaluated generators, missing trust and generator
failure with independently optional content, error and diagnostic fields. Consumers reconstructed
those states, including inspection status selection from diagnostic strings.

## Decision

- Store each Shell journal Action and its in-memory execution payload in one `PreparedShellStep`.
  Preparation appends complete steps in the existing resource order. Build the payload-free
  Action IR by projecting those steps, then execute the same ordered records. Keep Action validation,
  exact permission checks and initial journal publication before resource mutation; retain each
  applied-marker save, receipt boundary, lock lifetime and explicit recovery behavior.
- Use private `AppContentAssessment` variants for ready transformed bytes, not evaluated, trust
  required with local error, and evaluation failed with local error. Inspection projects the
  existing DTO fields and statuses from these variants. Preserve missing/partial/user-modified
  precedence, generator fallback content and transform order. Installation still preserves an
  installed file after generator failure; upgrade still reports failure while preserving it.
  New-file failure, manual-generator skipping, trust checks and execution gates remain unchanged.

## Consequences

Commands, output, Plan fingerprints and ordering, report DTOs, receipts and journal schemas remain
unchanged. Executable payload bytes remain in memory and never enter the journal. The pairing is
local to Shell execution, not a generic transaction abstraction or reusable approval authority.

Consumer regressions interrupt mixed cache/rendered/profile/launcher execution at each applied-marker
save, then verify the complete ordered journal, explicit recovery and a fresh retry. App regressions
exercise assessment through inspection and approved install/upgrade, preserving content, diagnostics,
execution counts, redacted generator errors and last-known-good destinations and receipts.
