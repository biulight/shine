# ADR 0146: Shared privileged file primitives and test fixtures

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Share the App manifest compatibility input matrix between the lifecycle reader, recovery receipt
reader and direct persistence reader. Keep consumer-specific assertions for error context,
read-only operations, raw recovery bytes and legacy receipt ordering. Move duplicate direct-reader
checks out of both consumer tests into the persistence owner's tests, including real IO failure.

Share installed-content hashing by App install strategy; callers pass their strategy directly.
Remove the file/receipt forwarding wrappers, which add no behavior or compatibility responsibility.
Desired JSON hashing remains distinct: absent managed keys and invalid content must retain their
existing installed-versus-desired behavior. Share only the CLI test event recorder; App and Shell
uninstall guidance assertions continue to exercise their own production handlers.

Stale destination admission and stale pruning also use the installed-strategy hash helper. Retain
their distinct missing-file policies and treat invalid JSON as a mismatch at both call sites;
neither caller propagates that parse failure or substitutes desired-content hashing.

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

The first regression set established acquisition ordering but did not directly observe the guard
through a suspended restoration. Add a shared Host probe that delegates filesystem behavior to
InMemoryHost and checks the guard at each read, metadata and privileged mutation. App and Sys
consumer tests suspend restoration, verify a competing lock cannot enter, then either resume or
cancel. Completion and cancellation release the lock; cancellation preserves the unrestored backup.
Additional fault injection covers failed install rollback (original write error retained) and
failed uninstall restoration (move error propagated, backup retained). These are existing
best-effort semantics, not new crash recovery. No production behavior correction was required.

Policy tests are separated into dry-run, install and uninstall scenarios so failures are localized.
Small local setup is retained instead of introducing another generic fixture. The shared suspended
restoration Host probe remains necessary: ordinary final-state assertions cannot show that the
lock stayed held while restoration was pending. Domain force/routing differences remain explicit.
