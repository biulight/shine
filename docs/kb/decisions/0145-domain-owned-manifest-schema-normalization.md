# ADR 0145: Domain-owned manifest schema normalization

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

App manifest readers repeated the same version gate in persistence, lifecycle, planning and
transaction/recovery receipt loading. Sys repeated its gate in persistence and planning.
Move each domain's normalization onto its manifest type; keep App and Sys rules independent.
Shell retains its existing normalizer, which also migrates launcher receipt fields.

Only legacy schema 0 becomes the current schema in memory. Unsupported schemas retain the exact
existing error. Parsing, missing-file handling, read errors, raw snapshot bytes and write-time
validation stay at the existing entry points. In particular, planning keeps bare TOML parse errors,
while lifecycle and App recovery readers keep their contextual parse errors. Do not replace those
loaders with a single parser or normalize their persisted bytes during review.

This does not change schema numbers, inner Sys receipt versions, receipt ordering, approval inputs,
lock ownership, journal ordering or recovery effects. No public API or serialized shape changes.

## Validation

Before extraction from `d095431`, three consumer tests passed across all six loading paths.
They cover absent files, unversioned state, explicit schema 0, current schema 1, future schema 2,
negative versions and malformed TOML. Assertions retain each reader's parse/error boundary,
original observation bytes and read-only behavior. Existing manifest roundtrip, receipt-version,
lifecycle and recovery tests remain applicable.

Expanding the six calls reproduces each original loading implementation. After extraction,
the all-feature workspace suite passed 1,721 tests with two opt-in tests skipped. One embedded
metadata list test was marked leaky in the full run and passed without that marker when rerun
alone. Workspace Clippy with warnings denied, formatting, typos, dependency bans/licenses/sources,
KB links and diff whitespace checks passed. Dependency checks retain existing duplicate-crate
and unused-license warnings. Validation ran on macOS; native Windows/Linux execution was not run.

The second review adds nonempty legacy receipts, preserved duplicate App receipt ordering and an
independently versioned Sys script receipt. Real filesystem read failures retain the distinct
planning/lifecycle/recovery error contexts and remain failures rather than missing-file defaults.
Recovery lock-wait coverage now includes a comment-only manifest edit: identical decoded receipts
still invalidate the previous raw-byte-bound approval under both lifecycle and privileged locks.
All seven focused tests pass on both an isolated `d095431` source snapshot and the current code,
using separate build directories. No production behavior correction was needed.
