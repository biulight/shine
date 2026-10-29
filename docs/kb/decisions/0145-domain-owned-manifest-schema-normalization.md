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

## Validation boundaries

Baseline comparison against `d095431` retains the original loading implementations. Consumer
regressions cover all six loading paths: absent files, unversioned state, explicit schema 0,
current schema 1, future schema 2, negative versions and malformed TOML. Assertions preserve each
reader's parse/error boundary, original observation bytes and read-only behavior.

Independent cases retain nonempty legacy receipts, duplicate App receipt ordering and an
independently versioned Sys script receipt. Real filesystem read failures retain distinct
planning/lifecycle/recovery contexts and remain failures rather than missing-file defaults.
A comment-only manifest edit during recovery lock wait still invalidates the previous raw-byte-bound
approval under both lifecycle and privileged locks, even when decoded receipts are identical.

Existing manifest roundtrip, receipt-version, lifecycle and recovery tests remain applicable.
Local macOS validation does not establish native Windows/Linux execution.
