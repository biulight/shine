# 0102 — Transformed Live Bun commands launch through one installed receipt

- **Status**: implemented; macOS/Linux full suites, three-platform real-Bun terminal and Windows launch/migration checks passed; Windows full-suite and Clippy baseline failures recorded
- **Date**: 2026-10-09
- **Evidence**: `core/src/runtime/{shell_launch,shell,launcher}.rs`, `cli/src/shells/deployment.rs`,
  `cli/src/env/{workspace,live_bun}.rs`, `tests/live_bun_launch.rs`

## Context

The old transformed Live Bun launcher renders through one Shine process and, with declared env,
starts a second `env run` process. The latter does not bind the installation directory. Even the
render call omits that binding for any directory named `.shine`. Replacing this template without
retaining its old bytes would invalidate receipt-owned update and recovery proofs.

## Decision

Eligible installed commands are Live, Bun, transformed, and not sourced. New receipts use
`launcher_format = "live-bun-v2"` plus absolute `launcher_config_dir`. Absent fields mean the exact
legacy format; unknown or inconsistent combinations fail. Unix, PowerShell and CMD templates bind
that root and pass one canonical target plus `--`-delimited argv to hidden `__shell-launch`.
Other templates remain unchanged. New CMD literals double percent signs and disable delayed
expansion and skip raw ownership metadata during execution; PowerShell and Unix use their native literal quoting.
PowerShell 5.1's native binder drops empty args and consumes `--`. The new PowerShell template
instead encodes the complete Windows argv (including embedded quotes and trailing backslashes)
for `ProcessStartInfo` with shell execution disabled. It starts one PATH-resolved Shine executable
and inherits stdio/cwd; it neither starts another shell nor changes legacy template bytes.

Shell manifest schema becomes 2. Readers normalize versions 0/1 only in memory and reject new-format
fields in old-schema containers. Inspection never rewrites state; selected approved lifecycle work
writes schema 2 without converting unselected legacy receipts. Both planner and executor use the same
schema normalization and resource reconstruction. The format and root participate in receipt
comparison, Plan observations, Action receipt conversion and recovery; format-only changes use the
existing `UpdateShellLauncher` transaction. Modified resources remain conflicts.

Action IR and Shell journal remain schema 1: their receipt grammar already rejects unknown fields.
New-format fields are serialized only when present, so legacy journal receipt bytes and semantics
remain supported. An old receipt reader rejects new fields even if interruption precedes manifest
commit. Legacy renderer and templates remain available; neither upgrade nor recovery silently
converts a pending journal. Downgrading the binary cannot reinterpret a schema-2 manifest.

CLI loads its existing layered Config once and builds the existing general Core runtime. Under the
cross-process operation lock, Core validates one unique installed target, runtime/mode/format,
transforms/env grammar, absolute source and bounded regular output paths; it refuses pending Shell
journals and renders that captured receipt. Preparation returns only script path, dependency enum
and env specs. The CLI drops the runtime before resolving declared values and awaiting Bun, without
calling `env run`, discovering workspaces, changing cwd, or running a version notification.
Encrypted-first values override only declared child targets; template rendering uses raw Config env.
Existing initialization/default backfill and installation-time Development trust remain unchanged.

Native Windows validation exposed stack overflow in the actual debug executable during startup
and installation. For Windows MSVC, the build script reserves 8 MiB for the Shine main thread;
pages commit as used. This applies to the shipped executable, rather than increasing only the
test-worker stack. Fixtures create isolated Windows roaming/local data directories alongside HOME.

## Consequences

- Only migrated eligible launchers fix directory binding; old and out-of-scope launchers retain
  their existing behavior until separately addressed.
- The lock ends after rendering. Another invocation or lifecycle operation can replace/remove the
  shared output before Bun reads it. No per-invocation source freeze is claimed.
- Rendering is not an env/launch transaction: later decryption/spawn failure does not undo output.
  A post-replacement persistence/mode failure can leave new bytes visible while execution fails.
- Declared dependency mode selects `--no-install` or `--install=fallback`; Live dependency bytes
  remain mutable. Shine neither installs packages nor owns Bun's cache.
- Empty-env launchers gain a waiting Shine process compared with direct Bun exec. Measure latency
  and resident memory separately; do not infer a speedup from process count alone.
- Unix child signal exits retain the existing `128 + signal` mapping. SIGINT/SIGTERM/SIGHUP sent to the waiting Shine PID are relayed to the direct child, which is reaped.
  Real-process tests cover inherited PTY input, process-group SIGINT, and parent-only SIGTERM.
  Linux/macOS real Bun 1.3.14 additionally passed controlling-terminal input, terminal-driver Ctrl-C,
  exit-code propagation and direct-child reaping with and without declared env. Windows CMD and
  PowerShell 5.1 passed equivalent ConPTY checks with both Shine and Bun confirmed gone after Ctrl-C.
  Windows outer-shell Ctrl-C statuses remain native (CMD 255, PowerShell 0); direct Bun under
  PowerShell returned the same 0. This work does not claim sandbox or descendant containment.
