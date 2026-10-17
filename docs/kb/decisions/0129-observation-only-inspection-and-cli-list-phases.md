# ADR 0129: Observation-only frontend inspection and CLI list phases

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Core App/Shell ordinary inspection already needed only filesystem observation, but frontend and
read-only adapter bounds still required mutation, privilege or process capabilities. Sys managed
inspection lived beside mutating drivers under broad bounds. CLI list/update/status combined input
capture, grouping, generator attention, diff reads and terminal output, making their distinct error
and output timing difficult to assess.

## Decision

- Narrow ordinary FrontendService and ReadOnlyFrontend App/Shell/Sys inspection to filesystem
  observation. Share the existing App report projection between selected static assessment and
  trusted explicit evaluation. Keep explicit evaluation's entry point, trust checks and execution
  bounds; the read-only adapter cannot call it. Keep safe diagnostics and local-detail boundaries.
- Move receipt-based managed Sys inspection into its own module. Keep desired input builders
  Host-free and narrow manifest inspection to observation. Preserve receipt-only status semantics,
  managed-only inventory compatibility, ordering, errors and no provider/version/live DNS probes.
  Public return types and persisted formats stay unchanged.
- Retain CLI entry points and split ordered query capture, pure projections and terminal rendering.
  Use named local query results and generator attention facts; pending lifecycle identity still
  filters update actions. Keep manual refresh commands distinct from upgrade targets.
- Preserve read counts and timing: update keeps both Shell captures, then App/Sys/missing-source
  queries before output; status prints Shell attention before App/Sys queries. Inventory retains
  its separate missing-App-source error policy. Keep detailed diff reads, empty/no-op paths,
  generator notices, post-render failure returns, wording and section ordering at their old boundaries.
  These local data types carry no approval or ownership authority.

## Validation and consequences

Consumer regressions use an observation-only Host for three-domain frontend/read-only inspection,
compare ordinary App reads and reports against the prior non-evaluated options path, and retain
safe manifest errors, category filtering, manual-generator unknown state and Sys recorded/missing
states. Compile-fail coverage keeps generator evaluation outside ReadOnlyFrontend.

Process-level list/update/status tests first run against the unchanged baseline binary. They cover
ordinary generator deferral, explicit trust rejection and evaluation, mixed upgrade/refresh hints,
receipt/destination preservation and Shell attention before later App errors. Existing projection
and frontend conformance suites remain applicable. Public commands, reports, Plans/fingerprints,
approval, trust, locks, receipts, journals and recovery contracts do not change.

Native Windows/Linux behavior requires separate platform execution; local CLI and virtual Host
coverage do not establish real external provider, administrator or remote behavior. No public
manual or portal change is required because commands and user-visible behavior remain unchanged.
