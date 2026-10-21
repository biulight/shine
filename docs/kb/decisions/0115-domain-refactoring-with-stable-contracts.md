# ADR 0115: Refactor domain phases behind stable contracts

- **Date**: 2026-10-16
- **Status**: Accepted

## Context

Large planner tests and lifecycle methods obscured domain ownership. App report producers maintained
parallel action/status/error fields. Workspace command handlers mixed formats, crypto, broker
snapshots and child execution. Sys resource dispatch repeatedly matched the full cross-domain Action
IR, while terminal grouping repeated string-based classification rules.

## Decision

- Group planner regressions by domain and contract under `planner/tests/`, retaining the shared
  harness in `tests.rs`. Preserve existing test names, assertions and platform conditions.
- Project internal `AppFileOutcome` variants into the unchanged App report DTO. Bind backup and
  failure details to their outcomes; retain the distinct install-preserved and upgrade-failed
  generator statuses and their existing local error channel.
- Split App install/removal into preparation, per-file execution and finalization. Preparation
  retains existing cache/generator effects. Keep outer lifecycle lock ownership, authorization
  timing, teardown-before-removal, per-resource receipt saves and receipt-before-journal commit.
- Separate App ownership preflight from generated/static planning, and Shell category cache,
  shared snapshot/sibling review and command convergence. Preserve observation order, short-circuit
  behavior, step/permission order and digest inputs. Reobserve after approval as before; never
  reuse pre-approval facts as execution authority.
- Keep workspace entry points as a facade over format/model, sealing, compilation, cache, broker
  snapshot and run modules. Use explicit imports for extracted modules. Local hybrid preferences
  remain outside broker snapshots; source precedence, complete source validation, plaintext guards,
  no-clobber publication and child supervision retain their contracts.
- Convert supported Sys Action IR variants to a private borrowed domain view for exhaustive
  resource dispatch. Keep persisted Action IR/journal schemas and receipt/approval checks unchanged.
  File, split-DNS, profile, observation and journal IO helpers remain domain-owned.
- Centralize CLI step presentation classification without adding Plan fields. Classification is
  terminal-only, preserves unknown diagnostics and never supplies approval or mutation authority.

## Consequences

Commands, output, Plan serialization/fingerprints, report fields, receipts and journal formats retain
their contracts. These changes introduce no cross-domain transaction engine. Regression coverage
retains interrupted execution, stale approvals, ownership conflicts and cancellation, and adds
outcome-detail compatibility and unknown-diagnostic/non-Sys-action boundary cases.

Review coverage also exercises App lock contention followed by approval invalidation before any
preparation effects, plus two-file install/removal interrupted at the second receipt save. Explicit
recovery preserves the first committed result, rejects replay of the stale lifecycle approval, and
permits a newly reviewed retry of the remaining work. Resource rollback remains per action, not a
transaction spanning all selected files.
