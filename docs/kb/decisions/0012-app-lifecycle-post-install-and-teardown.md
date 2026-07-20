# 0012 — App lifecycle: `post_install` hooks and artifact `teardown`

- **Status**: accepted
- **Evidence**: `cli/src/apps/hooks.rs` (`run_app_hooks`, `HookPhase`), `cli/src/apps/build.rs`
  (`handle_unbuild`, `run_teardown_for_uninstall`), `cli/src/apps/install.rs`,
  `cli/src/apps/uninstall.rs`, `cli/src/apps/metadata.rs` (`AppCategory.post_install`,
  `AppArtifact.teardown`), `shine app unbuild <app-id>`, `presets/app/surge/unbuild.sh`
- **Supersedes**: the "not auto-reversed" consequence of
  [ADR 0009](0009-app-artifact-build-explicit-command.md)

## Context

Two gaps surfaced after [ADR 0009](0009-app-artifact-build-explicit-command.md) shipped:

1. **Artifact side-effects could not be reversed.** `shine app build <id>` runs an
   `[artifact].script` (e.g. patching the active Surge profile's `#!include` lines) but records
   nothing — no manifest entry, no receipt. So `shine app uninstall` removed only the plain
   `Copy`-installed files and left the profile patch behind; ADR 0009 accepted this as a documented
   manual step.
2. **First install ran no hooks.** `post_upgrade` fires only on `shine upgrade` and only for changed
   categories, so a preset's `surge-cli reload` happened on a *later* upgrade but never on the very
   first `shine app install`.

The reversal problem has two shapes. A **generic receipt model** (like `sys/` drivers, which persist
a `SystemReceipt` and reverse it) would force Shine core to understand what an arbitrary script did —
exactly what ADR 0009 forbids, and exactly why `sys`'s own `Script` driver is *exempt* from reversal.
The alternative is a **symmetric teardown script**: the overlay owns both build and un-build, and
Shine only runs it.

## Decision

Add two lifecycle mechanisms, reusing existing machinery and the `allow_app_hooks` gate — no new
config key.

### `post_install` hooks

- `AppCategory` gains `post_install: Vec<AppHook>` (same `{ command, args, show_output }` shape as
  `post_upgrade`; single-table or array TOML).
- `run_post_upgrade_hooks` is generalized into `apps/hooks.rs::run_app_hooks(config, get_category,
  changed, HookPhase)`; `upgrade` passes `PostUpgrade`, `install` passes `PostInstall`. Identical
  semantics: run once per *changed* category, gated behind `allow_app_hooks` for external presets,
  failures non-fatal. `reinstall` = `handle_install(force = true)`, whose forced overwrite counts as
  a change, so `post_install` fires there too.
- Hooks still inherit only the parent env (no `SHINE_APP_*`/`[env]` injection) — that richer contract
  is reserved for artifact scripts, below.

### Artifact `teardown`

- `AppArtifact` gains `teardown: Option<String>` — a companion script, run with the **same** full
  `SHINE_APP_*` + `[env]` contract as `build` (so it can read the same `SURGE_PROFILE` etc.).
- Two entry points with deliberately different semantics:
  - **`shine app unbuild <id>`** (`handle_unbuild`) — explicit, symmetric to `build`: **not** gated
    by `allow_app_hooks`, and a nonzero exit propagates as a real error.
  - **during `shine app uninstall`** (`run_teardown_for_uninstall`) — implicit, so like the hooks:
    **gated** by `allow_app_hooks` for external presets, and **non-fatal** (a broken teardown must
    never block file removal). It runs *before* the file-removal loop so the script sees the same
    on-disk state `build` saw, and `--dry-run` prints the intended script without executing.
- Shine core still does not know what the script does — reversal logic lives entirely in the
  overlay's `unbuild.sh`, mirroring how `build.sh` works. The built-in `presets/app/surge/unbuild.sh`
  is an inert placeholder + commented reference example (the canonical overlay is private).

## Consequences

- `shine app uninstall surge` now reverses the profile patch (given the overlay's `unbuild.sh` and
  the `allow_app_hooks` opt-in for external presets), closing ADR 0009's manual-step gap. Reversal is
  still best-effort and script-owned, not a core-tracked receipt.
- A preset's reload/setup can run on first install via `post_install` without waiting for an upgrade.
- `shine upgrade` and `shine app install` stay side-effect-predictable: they never run artifact
  scripts; only the explicit `build`/`unbuild` commands and the uninstall-time teardown do.
- Adding either mechanism to a new app preset needs no Shine core change — just a `post_install`
  table and/or an `[artifact].teardown` script that honors the env contract.
