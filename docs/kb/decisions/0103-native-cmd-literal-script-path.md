# 0103 — Version the corrected native CMD script path

- **Status**: Implemented; Windows execution and migration tests require native Windows CI
- **Date**: 2026-10-09
- **Evidence**: `core/src/runtime/{launcher,shell,planner}.rs`

## Context

Legacy native CMD launchers apply PowerShell expression escaping to a double-quoted `-File`
argument, doubling literal apostrophes. Changing the only template would also change the bytes
used to prove receipt ownership during upgrade, uninstall and recovery.

## Decision

Windows native `.ps1` launchers use `launcher_format = "native-cmd-v2"` in schema-2 receipts,
without `launcher_config_dir`. The format preserves the original `-File` path's apostrophes.
Receipt decoding rejects this format for non-native or non-`.ps1` entries and rejects a config-dir
field. Absent format fields still reconstruct the exact legacy template. Planner and executor
select the same desired format; existing launchers change only through approved lifecycle work.

## Consequences

The PowerShell companion template and target ownership markers are unchanged. Portable tests
cover legacy/corrected template bytes and receipt validation. Windows-only tests exercise the
actual CMD-to-PowerShell invocation and the approved migration/uninstall path.
