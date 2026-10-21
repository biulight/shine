# ADR 0136: Test production asset and Config entry points

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Move resource-removal assertions to the approved Shell uninstall handler. Install through the
production handler before checking removal of managed launchers/cache and receipts, retention of
extra user files and configuration, and repeated whole-shell uninstall. Dry-run preserves cache,
manifest and profile bytes and creates no missing runtime tree. Preserve current selection rules:
a selected missing command is an error. Whole-shell `--purge` explicitly includes extra files in
the cache tree; ordinary removal preserves them. Do not transfer the old asset-name deletion
algorithm into the lifecycle owner. Delete `PresetAssetReader::remove_prefix` and `RemoveReport`.

Move prefix, collision, nested-directory and executable-mode tests to the actual `preset copy`
implementation, `copy_embedded_preset`. Copy accepts a canonical kind/category, not the old helper's
whole-kind prefix. Concurrent reader export tests call `handle_preset_export` with independent
Configs, preserving overlay-first export and pristine embedded copy. App destination fixtures use
`extract_embedded_prefix`, then distinct disk bytes and `from_installed_presets` to guard against
embedded fallback. Delete the unused overlay-aware `extract_prefix` adapter; retain `extract_all`,
`extract_embedded_prefix`, and their shared safe publication implementation.

Config file/key tests call the production `read_minimal_config_file`; the global-only initialization
test calls `load_global_runtime(true)` rather than project-discovering `load_or_init`. Delete the
old async file-reading helper and the unused `load_global_runtime_or_init` wrapper. Keep the
production dry-run loader and environment provenance resolution. Gate `installed_shell_dir` and
`env_override_source` with `cfg(test)`, limiting the former to the crate and the latter to Config's
module tree; installed snapshot management and provenance storage remain production code.

## Boundaries and validation

Establish the replacement assertions before removing the retired implementations. Run workspace
regressions, approval/read-only capability doctests, formatting, lint and dependency checks after
cleanup. Copy regressions check the exact selected path in reports and complete embedded bytes;
launcher removal checks `NotFound`, not an arbitrary metadata error. No command behavior,
compatibility path, Plan, approval, lock, receipt, journal, recovery
or extraction safety implementation changes. Local tests do not establish native execution on
other operating systems or real privileged/provider behavior. This is internal maintenance with
no public manual, portal or private-documentation behavior change.
