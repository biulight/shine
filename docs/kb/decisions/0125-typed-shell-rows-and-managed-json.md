# ADR 0125: Typed Shell rows and shared pure managed JSON rules

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

CLI Shell update matching and lifecycle targets reconstructed command identity from display labels.
Category grouping and upgrade/diff hints inferred status from symbols. Pure App JSON ownership
helpers lived in the lifecycle facade while planners, transaction execution and recovery depended
on them across module boundaries.

## Decision

- Preserve Core file status, category, command identity and ownership/missing-source flags in Shell
  rows. Keep the existing `ShellRow` and builder paths through re-exports. Project display fields
  once; use explicit selectors/targets and a typed presentation severity for subsequent decisions.
  Retain the legacy CLI terminal-component normalization for lifecycle targets from non-canonical
  receipt command names; selectors still retain the complete command identity.
- Retain all current symbols, text, installed filtering, attention precedence, category ordering,
  last-row tie selection and partial-install behavior. Keep source capture and inspection call
  order unchanged. Pending/Conflict/Preserved outcomes and their diagnostic/effect order retain
  their existing rules; this CLI type is neither execution intent nor approval authority.
- Extract JSON payload selection, merge/remove/restore, key comparison, parsing/serialization and
  subset hashing into a Host-free `runtime/managed_json` module. Point assessment, planning,
  execution and recovery at that owner. Preserve function bodies, error order and context, shallow
  top-level ownership, pretty bytes/trailing LF, FNV hash, and absent/partial/null distinctions.
- Retain filesystem observations, writes, backup/rollback, journal and receipt sequencing in their
  existing callers. Keep the key-owned recovery contracts of ADRs 0057 and 0063 unchanged.

## Consequences

Display changes cannot alter a Shell target, upgrade applicability or category severity. App JSON
helpers have a clear shared owner without importing Host or lifecycle execution capabilities.
Commands, output, Plans/fingerprints, receipts, journals, trust and recovery behavior stay unchanged.

Consumer regressions use approved Core Shell installation over an InMemoryHost, production
inspection-to-row conversion and list/update consumers. They bind display independence, partial
categories, severity/tie order, missing-source guidance, conflict preservation, stable outcome
identity and observation-only reporting. JSON regressions pin subset bytes/hashes, missing/partial/
null states, unrelated current values and error context/order; existing transaction/recovery suites
continue to exercise the same helpers through real consumers.
