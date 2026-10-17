# ADR 0134: Test production inventory and upgrade presentation

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Move managed Sys inventory filtering from the old `installed_managed_sys` method to the production
Frontend inventory request with `installed_only()`. Seed legacy receipts for managed/non-managed
items and two OS identities with distinct targets, then query each OS and assert its selected target
and unchanged manifest bytes. Distinct targets prevent inventory deduplication from masking a broken
OS predicate. Remove the unused runtime query and its CLI test adapter; keep `inspect_managed_sys`
and `SysInstalledRow` for production inspection.

Replace three Shell row-filter tests with a real approved virtual installation followed by
Frontend inventory and the CLI category projection. Cover current, updated, foreign-launcher,
receipt-without-launcher, legacy-launcher-without-receipt, uninstalled and missing-Preset states.
Check observation-only Host operations and unchanged receipt/launcher bytes. Preserve the current
production contract: available categories with receipts or existing launchers remain visible;
manifest-only missing Presets remain installed in inventory but are excluded from the simple Shell
list. Assert inventory flags as well as projected categories. Do not restore the old
test-only predicate based on status strings.

Test Shell link-upgrade summaries through the production approved upgrade handler and captured
reporter output. Real installation followed by a missing link and stale symlink exercises created
and updated counters in both verbosity modes. A subsequent no-op, including full output, emits no
Bin Links summary. Remove the separate test-only summary function; the production summary does
not include the old helper's `up to date` branch. Retain the installation summary renderer and test.
The symlink output fixture runs on Unix; the inventory matrix uses the cross-platform virtual Host.

Migrate 18 test call sites directly to the production targeted approved upgrade entry with the same
filter, pruning, verbosity and confirmation values. Delete the three App and three Shell wrapper
functions that had only test consumers, together with their re-exports. Keep prepared aggregate
upgrade and targeted approved handlers, including their fresh review/approval and execution logic.

Restrict seven Core helpers to `crate::runtime`: App cache reconciliation, Shell profile writing,
Shell desired-source lookup, effective transforms, cache reconciliation, empty-root removal and
Sys item preflight. Preserve their bodies, current lifecycle callers and compatibility paths.

## Boundaries and validation

Establish the migrated inventory/output regressions before deleting the old implementations.
Run workspace tests, approval/read-only capability doctests, formatting and Rust lint checks.
Plans, permissions, locks, receipt/journal formats and explicit recovery remain unchanged. This is
internal cleanup with no public command or manual behavior change. Native Windows/Linux and real
administrator/provider execution remain separate validation boundaries.
