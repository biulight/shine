# ADR 0139: Test production App inspection and Shell profile mutation

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Move the two App capability/partial-failure tests (three inspection call sites) to
`FrontendService::inspect_apps`, preserving the observation-only Host and assertions on deferred
generators, independent receipt updates and stale-approval rejection. Delete the observer-ignoring
`CoreRuntime::inspect_apps` forwarding method. Keep selected read-only assessment, explicit
`inspect_apps_with_options` evaluation and the shared report projector unchanged.

Move eight Shell profile mutation tests to production owners. Empty-source setup and idempotence
use CLI completion installation. Source wrappers, command-scoped setup, Windows PowerShell's two
profiles and removal use approved CLI install/uninstall. The legacy sentinel migration test first
installs actual source commands, then calls `install_shell_completion`, the same Core entry used by
CLI completion. It retains exact update status, surrounding content and wrapper assertions, and
also checks the receipt is unchanged. No migrated test supplies its own source-command list to a
profile writer. Delete the two test-only CLI append/remove wrappers and their imports.

Delete the uncalled test-only `install_bytes` and `uninstall_entry` wrappers and re-exports.
Their explicit-Host implementations and outcome types remain production dependencies of App/Sys
fallback paths, including backup protection and user-modification checks.

Restrict ten methods to `crate::runtime`: `install_shell_profile`, `remove_shell_config_blocks`,
five approved App file/JSON action executors, `commit_app_managed_file_operation`,
`run_app_generator` and `run_app_hooks`. Their bodies and production consumers remain unchanged.
The profile helpers still serve completion and lifecycle paths; the App helpers still serve
approved mutation, receipt commit, assessment, refresh and lifecycle hooks.

## Boundaries and validation

Validate migrated tests before deleting the old entry points, then run workspace tests,
capability-boundary doctests, formatting, lint, dependency and documentation checks. Compare
retained production bodies with baseline `562ab3e`; visibility and removal of unused forwarding
methods must not alter execution. Preserve compatibility reconstruction, Plan/approval checks,
lifecycle locking, receipt and journal schemas, transaction ordering and recovery.

This is internal ownership and test-entry cleanup, with no public command or stored-format change.
Only KB documentation changes; public manuals and the portal need no update. Local macOS tests do
not establish native Windows/Linux execution; the Windows dual-profile test remains platform-gated.

## Follow-up review

Comparison with `562ab3e` confirms the ten restricted methods retain their bodies. CLI production
install/uninstall/completion handlers and retained App inspection/file operations are unchanged;
only the unused forwarders, test imports and public visibility differ. Their callers still reach
snapshot-bound approval, lifecycle locks, receipt-before-journal-commit and recovery owners.

The migrated App tests call Frontend inspection, which shares `inspect_apps_read_only` with the
CLI's ordinary options-based inspection; the capability test still has an observation-only Host.
Shell tests use dispatch's approved handlers and completion entry. Review found one fixture gap:
installing source commands in the legacy sentinel test also pre-populated the managed profile,
so wrapper assertions could pass without completion writing it. Remove that profile before the
completion call and assert `profile_updated` as well as recreated wrappers, refreshed sentinel,
preserved surrounding content and byte-identical receipt. Production code needs no repair.
