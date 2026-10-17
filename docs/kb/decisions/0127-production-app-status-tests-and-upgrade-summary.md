# ADR 0127: Production App status tests and named target upgrade summaries

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

CLI App status tests retained an independent assessment implementation after production inspection
moved into Core. Some assertions had drifted: missing source metadata and ordinary manual-generator
inspection no longer had the helper's up-to-date fallback. The self-install module also combined
four distinct use cases and represented target upgrade results as a five-position tuple.

## Decision

- Establish migrated scenarios against the unchanged production FrontendService inspection path
  before removing duplicate CLI assessment/hash helpers. Fixtures only capture presets and seed
  receipts/current bytes. Retain CLI row presentation and real-host refresh coverage; ordinary
  inspection must remain observation-only, with explicit evaluation separate from mutation.
- Keep `self_install.rs` as the stable handler facade. Separate configuration/release checks
  (including opt-in generator evaluation), release installation, local binary publication/synchronization
  and configuration upgrade orchestration.
  Move existing tests beside their owners without changing their assertions or platform guards.
- Replace only the target-upgrade tuple with private `TargetUpgradeSummary` constructors. Preserve
  App category deduplication and auxiliary exclusions, Shell category/cache/PATH accounting, Sys
  resource counting, authoritative report failure totals and App restart hints. Do not merge the
  aggregate summary's distinct counting and error policy into this type.
- Preserve compatibility routing, target normalization, App-only pruning, review and preparation
  order, consumed approvals, fresh Core Plan/permission checks, locks and all execution adapters.
  Preserve local binary staging, absolute destination persistence, same-source checks, administrator
  serialization, Unix elevation and Windows manual synchronization behavior.

## Validation and consequences

Regressions exercise actual status/lifecycle projections for missing, modified, new and relocated
files, managed JSON keys, deferred manual generators and explicit refresh identity without receipt
or destination mutation. Domain summary tests retain category-versus-resource units, cache/hook
exclusions and non-fatal hook behavior. Process-level target upgrade tests retain consent, category
suffix normalization, unrelated-category preservation, no-op output/receipt stability, user edits,
invalid Sys selectors and App-only stale pruning. Review regressions also exercise real targeted
upgrade exit status for a non-fatal hook versus a fatal generator failure, restart hints, changed-code
trust rejection and retained last-known-good receipts. Missing-source tests assert the typed metadata
error; explicit evaluation tests check that old/new destinations and the receipt are not written or
removed.

No command, public DTO, Plan/fingerprint, permission, receipt, journal, trust format or output wording
changes. Native Windows/Linux execution, real release downloads and administrator escalation require
separate platform/external validation; virtual inspection tests do not establish those outcomes.
