# 0161 — Bounded tail capture for Sys installer logs

- **Status**: Accepted
- **Date**: 2026-10-31
- **Extends**: ADR 0100

## Context

Normal APT output can exceed the bootstrap 64 KiB log budget. Treating retention overflow as a
process failure kills package configuration and leaves the system partially installed. Raising the
limit only postpones the same failure; unbounded capture would remove the memory protection.

## Decision

Add explicit `ProcessIo::CapturedTail` for package and script Sys installers. Each pipe continues
to drain while retaining at most its configured tail budget. `ProcessOutput` records whether any
bytes were discarded, so the existing final diagnostic report can announce truncation even when
only one stream overflowed. The combined bootstrap report retains its existing 64 KiB budget.

Default captured output, including detection and generators, still fails on overflow. All modes
retain timeout, cancellation, failed-child and process-group cleanup. Installation success remains
based on exit status followed by the existing detection and receipt flow.

## Consequences

Large normal logs cannot themselves interrupt package installation. Earlier diagnostics can be
lost, explicitly reported as truncated; stdout/stderr tails are not a chronological merged log.
This does not resume interrupted package operations or change installation authorization.
