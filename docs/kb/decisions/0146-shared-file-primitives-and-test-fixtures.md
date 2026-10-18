# ADR 0146: Shared privileged file primitives and test fixtures

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Share the App manifest compatibility input matrix between the lifecycle reader, recovery receipt
reader and direct persistence reader. Keep consumer-specific assertions for error context,
read-only operations, raw recovery bytes and legacy receipt ordering. Move duplicate direct-reader
checks out of both consumer tests into the persistence owner's tests, including real IO failure.

Share installed-content hashing by App install strategy, retaining the file/receipt entry points.
Desired JSON hashing remains distinct: absent managed keys and invalid content must retain their
existing installed-versus-desired behavior. Share only the CLI test event recorder; App and Shell
uninstall guidance assertions continue to exercise their own production handlers.

Extract App/Sys privileged byte installation and removal into private `runtime::privileged_files`.
The helper holds the host admin lock through comparison, backup, write/remove and failure restore.
Domain callers retain lifecycle approval, receipt persistence, dry-run and non-privileged routing.
App retains force behavior; Sys passes no force for privileged operations, but its non-privileged
installer still forces writes. Keep domain-specific diagnostic contexts and the existing best-effort
backup restoration behavior, including propagation of the original write failure.

This is not a shared journal transaction engine. Plan fingerprints, ownership rules, manifest and
journal schemas, lock ordering, recovery dispatch and public output remain unchanged. No public
manual behavior changes.

## Validation

Before extracting privileged operations, add and run two domain consumer regressions against the
original implementation. Cover App dry-run without host calls, force versus unchanged content,
modified-file preservation, forced removal/backup restore, missing files, injected write failure
with backup restoration, and Sys privileged versus non-privileged behavior. Retain the existing
approval, lifecycle concurrency, receipt and recovery coverage.

Source comparison against `c114524` confirms all four privileged operation bodies match after
substituting the caller's force policy and diagnostic context. The pre-extraction domain tests
pass before moving those bodies; this comparison does not substitute for native platform tests.

Final macOS validation passed 1,728 all-feature workspace tests (two opt-in tests skipped), eight
compile-fail documentation tests, all-target/all-feature Clippy with warnings denied, formatting,
spelling, dependency policy and KB relative links. The first sandboxed workspace run stopped at
an SSH local-socket permission error; the complete rerun with local sockets permitted passed.
Dependency policy retains existing duplicate-crate warnings. Native Windows/Linux execution was
not performed. No commit or push was made.

## Second review

Compare against the pre-refactor `c114524` implementation. The manifest input matrix and all
existing CLI uninstall test bodies are unchanged apart from the shared recorder and its clear
method. Direct-loader checks now live at the persistence owner; lifecycle and recovery readers
still assert their distinct diagnostics, raw bytes, legacy entries and read-only behavior.
Installed hashing retains its exact strategy body and desired hashing remains separate.

The first regression set established acquisition ordering but did not directly observe the guard
through a suspended restoration. Add a shared Host probe that delegates filesystem behavior to
InMemoryHost and checks the guard at each read, metadata and privileged mutation. App and Sys
consumer tests suspend restoration, verify a competing lock cannot enter, then either resume or
cancel. Completion and cancellation release the lock; cancellation preserves the unrestored backup.
Additional fault injection covers failed install rollback (original write error retained) and
failed uninstall restoration (move error propagated, backup retained). These are existing
best-effort semantics, not new crash recovery. No production behavior correction was required.

All four domain privileged tests pass against an isolated `c114524` source snapshot with the new
tests added and production code unchanged. The final current workspace run passed 1,730 tests
(two opt-in tests skipped), including the same four regressions. All-target/all-feature Clippy
with warnings denied, formatting, spelling, KB relative links and diff whitespace checks passed.
The second review ran on macOS; native Windows/Linux behavior remains unverified.
