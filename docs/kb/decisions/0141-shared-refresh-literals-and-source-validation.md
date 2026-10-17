# ADR 0141: Shared refresh selection, literal encoding and source validation

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Remove the CLI package's unused direct `jsonc-parser` dependency; Core retains the JSONC transform
and dependency. Migrate the eight status-row test calls to the production options-based entry with
`run_generators = false`, and the two Sys template assertions directly to Core parsing. Delete the
two old row forwarders and the test-only Sys manifest adapter; keep the assertions and fixtures.

Share generated-file selection and exact source/destination receipt matching in App's
`refresh_selection.rs`. Preserve declaration order, selected versus batch missing-file policy,
error text and each caller's read/error order: planner candidate selection precedes manifest IO,
while execution loads the manifest first. Both compute fresh selections. The approved entry still
holds the App lifecycle lock before replanning and validating approval; administrator consent,
generator trust, per-file continuation, final manifest save and hooks retain their owners.

Share POSIX and PowerShell literal encoders in Core, including a distinct PowerShell contents-only
encoder for templates that supply delimiters. Preserve exact launcher/SSH/profile bytes, legacy
CMD reconstruction, Fish semantics, path normalization and home expansion. SSH rejects NUL before
encoding. CLI argument display keeps its own safe-character policy. This internal cross-crate
module is hidden from generated documentation and adds no execution capability.

Share sealed-declaration checks and full decrypted-key equality in workspace `source_payload.rs`.
Local compilation, broker snapshot admission and hybrid-cache preflight retain sealing guidance;
broker decryption retains its shorter error. Metadata-only paths never decrypt. Sealing retains
the low-level decoder for pending sources. Local include-secrets policy, complete source validation,
precedence, broker declared/release sets, filtering and zeroization remain with existing owners.

## Boundaries and validation

Establish new selection/lock and secret-validation regressions on the old implementation before
extraction. Retain production tests for last-known-good output, administrator refusal, receipt-save
failure before hooks, stale approval, lifecycle serialization, transactions and explicit recovery.
Compare baseline/current output for Plans, receipts, Host traces, launcher/profile bytes, env export
and nested/encoded SSH commands, normalizing only random execution-directory UUIDs.

Plan/permission fingerprints, lock lifetimes, receipt/journal schemas, commit/recovery ordering and
secret formats remain unchanged. Refresh still has final receipt persistence after per-file writes;
this does not add crash rollback. There is no public command, output or manual behavior change.
Native Windows/Linux execution, real remote hosts and cryptographic hardware remain separate from
local tests, controlled subprocess fixtures and cross-target compilation.

The local macOS validation compared 269 baseline/current records with no differences after the
UUID normalization above. The initial extraction's all-feature workspace suite passed 1704 tests (two skipped),
including four added regressions; all eight compile-fail documentation tests passed. Formatting,
all-target/all-feature Clippy, dependency policy and spelling checks passed. The baseline suite
had one leaky-test warning; the initial extraction run did not reproduce it. Linux Core cross-target checking
passed. Windows Core cross-target checking passed with an existing unused-variable warning in
`launcher.rs`; full-workspace Windows checking
was blocked in `aws-lc-sys` by missing Windows C headers on the macOS host. Dependency-policy
checks retained existing duplicate-dependency and unmatched-license-allowance warnings.

A second baseline review added consumer regressions for unrelated receipts changed while refresh
waits for its lifecycle lock, and valid/corrupt/future-schema journals appearing during that wait.
The valid journal comes from a real approved static install with failed receipt persistence;
refresh leaves its resource and journal untouched until separately approved explicit recovery.
The failed-refresh-save regression also asserts that refresh creates no recoverable journal.
A seal regression retains old sealed values, replaces pending values and drops retired payload
keys, then reads the result through both broker and local consumers. Broker metadata admission
rejects the pending source without invoking the backend. These eight focused tests (including
the earlier refresh/source-validation contracts) also pass against the pre-extraction production
implementation at `e8f9c85740fac8f24f73fbc926932d2d1949b654`. No production correction was needed.
The second review's full workspace run passed 1707 tests (two skipped), with one leaky-test warning
on the unchanged `preset_commands::tests::copy_creates_nested_directories`; three isolated reruns
passed without that warning. All-target/all-feature Clippy, formatting, spelling, KB links and
Linux/Windows Core cross-target checks passed; the existing Windows unused-variable warning
remains. Cross-target checking does not establish native Windows/Linux execution.
