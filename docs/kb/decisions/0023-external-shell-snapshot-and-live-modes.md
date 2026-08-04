# 0023 — External shell presets default to snapshots with explicit live mode

- **Status**: accepted
- **Evidence**: `cli/src/shells/deployment.rs`, `cli/src/bin_links.rs`,
  `external_shell_mode` in `config.toml`

## Context

An external presets directory was both a source of desired state and, for raw shell commands, the
installed executable location. Raw files changed immediately, transformed shell files required
`shine upgrade`, and app files followed the normal update/upgrade lifecycle. Users therefore had
to understand transforms and runtimes to predict when an edit took effect. A checkout or branch
switch could also change an installed command without an explicit apply step.

## Decision

External presets select the desired source only. Shell deployment defaults to `snapshot`: install
and upgrade materialize the effective category under `<shine_dir>/installed/shell/`, and launchers
target that Shine-owned snapshot or its rendered output. `shine update` remains read-only and
compares the active source with the installed snapshot.

Preset developers may explicitly choose `external_shell_mode = "live"` or
`shine preset link PATH --live`. Raw commands target their external source. Transformed commands
use a managed launcher that invokes the hidden, manifest-constrained renderer before execution.
`needs_source` launchers detect sourcing and source the rendered result so changes remain in the
parent shell. Content and current env values apply on the next invocation; deployment metadata
still requires upgrade.

`shell-manifest.toml` records installed deployment metadata but never environment values. Runtime
rendering accepts only a canonical manifest target and can write only below the configured
rendered directory. A render failure leaves the last good file intact and fails the invocation
rather than silently running stale content.

## Consequences

- The default app/shell mental model is uniformly `update` then `upgrade`.
- Existing direct external links are reported as updates and migrate only during upgrade.
- Live mode has an extra Shine startup for transformed commands; raw live commands remain direct.
- External source and overlay directories remain user-owned and are never modified or removed.
