# ADR 0130: Refresh, receipt preparation and SSH construction boundaries

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Explicit App refresh mixed receipt selection, authorization, per-file generation/install and final
persistence/hooks. Shell preparation exchanged receipt changes as position tuples and combined
embedded-cache and shared rendered-resource preparation in the facade. SSH session orchestration
also contained pure option parsing and POSIX/PowerShell command construction.

## Decision

- Keep the approved App refresh entry point and caller-owned lifecycle lock/fresh Plan validation.
  Keep selected-file records and the per-file helper, but orchestrate selection, application and
  completion directly. Single-caller prepared/applied transport states added no invariant and are
  removed. Preserve exact receipt source/destination
  admission, selector/default order, administrator authorization before generation, generator trust
  and environment contracts, per-file failure continuation, result projection and force policy.
  Save the shared manifest once after changed files, then run hooks. Receipt-save failure still
  skips hooks and preserves the old durable receipt alongside already-applied file writes; this
  refactor adds no journal, rollback or per-file durable commit.
- Replace only local Shell receipt transition/removal tuples with named fields and one V1
  conversion. Preserve target identity, previous/desired receipt, optional absence and ordered
  resource/receipt projection. Leave persisted DTOs, validation, permissions, fingerprints,
  late-state rechecks, journal/receipt markers, guards and recovery dispatch unchanged.
- Move embedded source/cache preparation and shared rendered-file preparation into separate
  Shell modules. Preserve overwrite/preserve behavior, source/mode and transform selection,
  shared-output conflict checks, report ordering and the lifecycle's existing preparation effects.
  Share existing desired-delivery/receipt helpers without caching pre-approval observations.
- Move SSH option-boundary parsing/transport argv and pure remote wrapper construction into
  separate modules. Return a named private parsed result. Preserve all raw option tokens and
  ordering, destination/command selection, quoting, encoded script bytes, error literals and
  platform guards. Keep forwarding resolution, session ownership, broker setup and authorization,
  PID handoff, child supervision, draining, cleanup and exit propagation in the existing facade.

## Validation and consequences

Approved refresh regressions exercise administrator refusal, file failure continuation, one final
manifest save before hooks and failed-save behavior; existing selector/force/exact-owner and
lifecycle-lock suites remain applicable. The added refresh regressions were established against
the unsplit refresh implementation before extraction.

Shell consumer coverage binds preserved embedded source bytes and modes, both commands consuming
one rendered destination, ordered payload-free V1 journal receipts, failed receipt persistence,
explicit recovery and fresh retry. Existing mixed-resource preparation/apply/commit/recovery suites
retain previous-receipt and foreign-resource checks.

SSH CLI stub coverage preserves repeated option order and user ControlMaster settings, literal
forwarded values/remote arguments, encoded Windows payloads, POSIX-only reverse forwarding, cleanup
and nonzero child exit. Existing real-shell quoting, parser, broker and cancellation tests remain.
Native Windows/Linux execution, physical TTYs, remote hosts and real administrator operations
require separate validation. Public command/report, Plan/approval, receipt, journal, trust and wire
formats remain unchanged, so the public manual and portals require no behavior update.
