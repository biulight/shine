# ADR 0138: Test production inspection and explicit CLI entry points

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Move the two generic runtime harness regressions to `FrontendService::inspect_apps` and
`FrontendService::inspect_shells`. The App fixture seeds a receipt and its actual destination,
then checks current and desired content, update status and the Upgrade operation. The Shell
fixture checks a declared, uninstalled command and the Install operation. Both retain captured
inputs and an in-memory Host, and assert that inspection performs only reads. Use native absolute
fixture roots without creating real files; the Shell boundary fixture explicitly models Linux
and Bash. `RuntimeContext::isolated` otherwise chooses the host's default Shell independently of
the captured platform, which filters out this fixture's `.sh` source on Windows.
Delete `inspect_snapshot` and its `ResourceInspection` / `RuntimeInspection` DTOs: generic
root-plus-logical-path byte comparison is not the production domain inspection contract.

Move CLI tests to the same `*_approved` handlers used by dispatch, passing `yes = true` explicitly
where the retired wrappers supplied it. Remove eight test-only convenience wrappers: App install,
uninstall, build and refresh; Shell install and uninstall; and top-level Shim install and uninstall.
Remove the uncalled Sys uninstall wrapper as well. Preserve the production handlers and their
review, trust, snapshot approval, result reporting and completion checks. Test-only result/report
adapters that call those production owners remain outside this cleanup.

Delete the unconsumed `LifecycleRequest`, `ResourcePlan` and `DomainInspectionReport` types and
their exports. Keep domain-specific requests, `PlanV1`, `InspectionReportV1`, receipt and journal
types, and version compatibility aliases.

Restrict five helpers to runtime scope: `shell_deployment_source_path`, `shell_rendered_path`,
`link_is_current_with_host`, `validate_app_category_snapshot` and `sys_install_requires_admin`.
Keep their bodies and production consumers unchanged; remove unnecessary public re-exports.
In particular, administrator inference remains shared by bootstrap and permission planning.

## Boundaries and validation

Validate the migrated tests before removing the retired entry points. Run workspace tests,
capability-boundary doctests, formatting, lint, dependency and documentation checks after cleanup.
Compare production bodies against the pre-cleanup baseline in addition to searching all source
and test callers. Shell profile installation, completion, legacy reconstruction, broker behavior,
approval validation, lifecycle locks, receipts and recovery remain intact.

This changes internal ownership and test entry points, not public commands or stored formats;
there is no public manual or portal change. Local tests do not establish native Windows/Linux
execution or real encryption hardware/provider behavior.

## Follow-up review

The review against `d1bb72f` checked all 107 migrated CLI call sites: only the callee and
explicit `yes = true` changed, preserving arguments, fixtures and assertions. Removing the nine
forwarders leaves the same production handler bodies. The five restricted helpers retain their
bodies; administrator inference still feeds bootstrap and permission planning. Dispatch, approval,
locks, receipt/journal formats, compatibility and recovery implementations are unchanged.

The App harness reaches `inspect_apps_read_only`, also used by the CLI's ordinary
`inspect_apps_with_options` path, and verifies a real receipt/destination relationship. The Shell
harness calls the same Frontend inspection entry as CLI collection. Existing binary dispatch tests
retain refusal without consent, user-edit protection and receipt-driven removal after source loss.

On macOS with cargo-nextest 0.9.91, separately recompiled baseline/current workspace runs each
passed 1700 tests with the same two opt-in tests skipped and no leak warning. The previously
flagged `embedded_install_dry_run_previews_cache_without_extracting_it` test passed ten isolated
runs on each revision without a leak warning. Its body and production call path are unchanged.
There is no evidence tying the earlier warning to this cleanup, but its root cause remains
unconfirmed because it did not reproduce; no leak threshold or test isolation was weakened.
Nextest's [leaky-test detection](https://nexte.st/docs/features/leaky-tests/) concerns inherited
stdout/stderr handles remaining open after test exit, rather than Rust heap-leak detection.
