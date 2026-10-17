# ADR 0144: Share ordered App recovery observations

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Five App recovery planning branches repeat the same destination/rollback reads and snapshot
registration: file update, ordinary/forced file removal, JSON merge and JSON removal. Share this
sequence in `recovery_plan.rs`, restricted to `FileSystemObservationHost`.

Read the destination first, then rollback, before registering either observation. Preserve the
`destination:<action_id>` and `rollback:<action_id>` labels and existing byte/mode/kind identities.
Keep error short circuits and partial digest registration unchanged. Forced removal observes its
optional persistent backup only after this pair succeeds. Three-file backup removal and relocation
retain their own observation sequences; recovery apply still performs fresh live reads.

Receipt conflict checks, action-specific assessments, permissions, Plan/approval validation,
lifecycle and privileged locks, receipt markers, journal formats and reverse recovery execution
remain with their existing owners. No observation result is cached across review and apply.

Preset migration rendering also reuses `preset_report::count_phrase`; remove its identical local
copy while retaining exact singular/plural wording. Historical migration and receipt readers remain
because production callers still use them. GPG/age recipient validators stay separate because their
whitespace acceptance differs.

## Validation

Before extraction from `a8bcc0c`, two new consumer regressions passed for all five recovery actions.
They capture fixed state digests and exact read traces, and verify that duplicate observation labels
fail only after both reads, before optional backup reads or action assessment. The captured digests
remain permanent expectations. Existing recovery tests cover missing/non-file resources, modified
bytes/modes, receipt gaps/commit markers, stale approval, lifecycle locks and explicit rollback.

Expanding the five helper calls reproduces the three original planning files byte-for-byte;
the reused count phrase function body also matches the removed implementation exactly.
After extraction, all-feature Core and CLI suites passed 651 and 1,066 tests respectively
(two opt-in CLI tests skipped). One CLI dry-run test was marked leaky in the full run and passed
without that marker in an isolated rerun. Workspace Clippy with warnings denied, formatting,
typos, dependency bans/licenses/sources, KB links and diff whitespace checks passed. Dependency
checks retain existing duplicate-crate and unused-license warnings. Validation ran on macOS;
native Windows/Linux execution was not performed.

The pre-commit second review extends label-conflict coverage to partial registration at the
rollback label and subsequent optional backup reads. A third consumer regression holds each
lifecycle/privileged lock in turn, changes destination bytes, rollback bytes or receipts while
recovery waits, and verifies stale approval rejection with no mutation of files, receipts or the
journal. All three regressions pass both on an isolated `a8bcc0c` source snapshot and the current
implementation. No production behavior correction was needed.
