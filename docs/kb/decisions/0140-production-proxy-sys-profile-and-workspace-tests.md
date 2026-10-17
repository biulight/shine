# ADR 0140: Test production Proxy, Sys profile and workspace compilation

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Move Proxy launcher/quoting/conflict and rule replacement tests, including uninstall fixture setup,
to `install_proxy_files`, the platform-explicit production owner called by `install`. Preserve
Windows companion coverage without invoking a Windows shell. Check conflict failures leave rules
and receipts absent or byte-identical. Exercise global/project configuration privacy through actual
installation, `set_enabled` and `uninstall_for_platform`; reload configuration through the production
read-only loader under the environment guard. Global uninstall removes global rules, launchers and
the receipt but preserves project overrides. Do not revive test-only project-rule deletion semantics.
Remove `install_shims_for_platform`, `upsert_rule` and `remove_rule`. Keep the shared writer, ownership
checks, lock ordering, before/after-visible-write failure injection and reverse rollback tests.

Move five Sys sentinel tests through `update_sys_shell_profiles` with explicit shell/config inputs
and `RealHost`. Keep empty-file, idempotence, CRLF, leading BOM and legacy sentinel assertions at
real routed paths. Move two phase EOL tests to `install_sys_profile_phase_with_template` with an
explicit Host and request; retain `template_override: None` so file observation and merge selection
stay covered. Delete the two ambient-Host test wrappers, not the reconciliation or compatibility
implementations. Keep platform routing, active/base preservation and pending-artifact handling.

Move the workspace ordered-source regression to `capture_sources` followed by
`compile_captured_sources`, the production run path. Remove `compile_sources` and its test-only
import. Keep export's separate include-secrets policy, complete source validation, effective binding
precedence, decryption and sensitive-value ownership unchanged.

Restrict five Sys helpers to runtime scope: `split_dns_receipt`, `apply_split_dns`,
`remove_split_dns`, `SysInstall::success_status` and `SysInstall::success_hint`. Remove only the
public receipt-factory re-export. Production planning, inspection, transaction, bootstrap and
receipt-only lifecycle consumers remain intact.

## Boundaries and validation

Establish migrated regressions before deleting the six wrappers. Compare retained production
implementations with baseline `58b621c`, then run workspace tests, capability-boundary doctests,
formatting, lint, dependency and KB checks. Plan/approval semantics, locks, receipts, rollback and
recovery implementations must remain unchanged. Proxy rollback still covers reported failures
within a process; this cleanup adds no durable crash-recovery guarantee.

No public command, output or stored-format behavior changes, so only the KB changes. Local macOS
checks cover Windows-shaped files and quoting assertions, not native Windows/Linux execution.
