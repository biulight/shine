# 0153 — Converged upgrade ends after read-only planning

- **Status**: Accepted
- **Date**: 2026-10-24
- **Evidence**: `cli/src/dispatch/before_config.rs`, `cli/src/self_install/config_upgrade.rs`,
  `cli/src/lifecycle_plan.rs`, `core/src/plan.rs`

## Context

A no-op upgrade skipped its prompt but constructed approval and entered prepare/execute anyway.
Config initialization and advisory workers/cache writes preceded planning. No new user option is
needed to make an already-converged upgrade end without side effects.

## Decision

- Except for explicit `--pull`, observe convergence before ordinary configured dispatch. Use
  read-only config loading and the existing Core review service, never human approval construction.
- Share upgrade target/request construction with execution, preserving category suffix normalization,
  App-only stale pruning, Shell/App/Sys batch order and resolved input identities.
- Core requires a ready Upgrade Plan with only `None` steps, no required permissions, code boundaries
  or analysis work, and no exceptional diagnostics. Manual-refresh notices alone remain ordinary
  no-ops. Do not classify convergence from terminal filtering, content hashes or changed-row counts.
- On positive convergence render the existing no-op result, with full Plan details when requested,
  then return. Do not initialize state, spawn advisory workers, fetch releases, create approval,
  prepare runtimes, acquire execution locks, or persist receipts/journals.
- Managed Sys upgrade planning includes every recorded managed item, matching execution selection.
  `profile_enabled` records Shell integration intent, not whether a managed-file receipt is installed.
  Available but unrecorded items remain excluded from aggregate upgrade.
- Windows ownership probes compare the native path spelling emitted by shim markers, including
  canonical verbatim roots. Component containment, non-Unicode refusal and foreign-pair protection
  remain mandatory; this does not change source, receipt, trust or approval identities.
- All other cases retain the original route, including compatibility guidance, trust/code consent,
  exact permissions, source/state/config freshness and final lock-protected apply validation.
  Preflight failure cannot imply convergence. No Plan, fingerprint, approval or storage schema changes.

## Consequences

Changed upgrades incur one extra observation pass; this keeps the existing mutation path intact
instead of reusing stale preflight observations as approval. No-op upgrades avoid all lifecycle
execution. `--yes` and display flags do not change convergence; explicit `--pull` still synchronizes
sources before upgrading. Skipped manual generators are not evaluated to discover extra work.

Regressions cover empty state, all three domains and aggregate upgrade, byte/timestamp/tree stability,
output modes, advisory bypass, snapshot-only consent, profile-only repair, stale pruning, external
code trust and conservative rejection of maintenance, preservation, permission-only, blocked,
analysis-bearing and unknown Plan states. Current Shell delivery needs no trust re-enrollment.
