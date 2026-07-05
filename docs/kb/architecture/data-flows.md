# Data Flows

End-to-end flows that span multiple modules and are not visible in any single file. For the
module map and per-command routing table, see [`AGENTS.md`](../../../AGENTS.md) — this file only
records the cross-module sequences and their gotchas.

## App install (`shine app install <category>`)

`cli/src/apps/mod.rs` orchestrates:

1. **Metadata** — `apps/metadata.rs` parses `presets/app/<category>/shine.toml` (`dest`,
   `transforms`, `requires_admin`, …). Legacy categories without `shine.toml` (git, starship)
   use `apps/annotation.rs` to read a `shine-dest:` comment from the file itself.
2. **Transforms** — `apps/transforms/` applies `jsonc-to-json` and/or `template` (`@@VAR@@`
   substitution from the `[env]` config table) in declaration order, in memory, before writing.
3. **File ops** — `apps/file_ops.rs` backs up any pre-existing user file to `<name>.shine.bak`,
   then writes the (transformed) content to `dest`. Destinations with `requires_admin = true`
   (e.g. `/etc/docker/daemon.json`) go through the sudo path, serialized by a cross-process
   advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as mutex).
4. **Manifest** — `apps/manifest.rs` upserts an `AppEntry` into `~/.shine/app-manifest.toml`,
   recording dest, content hash, strategy, and **`requires_admin`** (must persist — uninstall
   routes on it; see lessons entry 2026-07-04).
5. **Report** — `apps/report.rs` prints the outcome.

## App uninstall

Reverse of install, driven entirely by the manifest — never by re-scanning presets:

1. Look up the `AppEntry` by dest in `~/.shine/app-manifest.toml`.
2. Remove the installed file (sudo path if the entry has `requires_admin = true`).
3. Restore `<name>.shine.bak` if one exists.
4. Remove the manifest entry.

## App upgrade (`shine upgrade`)

`apps/upgrade.rs::handle_upgrade_installed` re-applies presets (including re-running transforms
with the *current* `[env]` values) to every manifest-tracked install, and cleans up stale entries
whose preset no longer exists. `env/upgrade.rs` does the same for env-templated content. This is
why changing an env var requires `shine upgrade` to take effect in installed files.

## Shell install / uninstall

Documented in `AGENTS.md` § "Key data flow". Summary: extract embedded assets →
symlink executables into `~/.shine/bin/` (`bin_links.rs`) → append a sentinel-guarded PATH block
to the shell config (`shells/profile.rs`). Uninstall removes only shine-managed symlinks/files
and deletes the sentinel block precisely.

## Sys bootstrap (`shine sys init`)

Documented in `AGENTS.md` § "Sys preset flow". Key cross-module point: `sys/execution.rs` runs
`init.sh <item_id>` once per selected item and parses `SHINE_SYS_STATUS\t<state>\t<detail>` lines
from script stdout into the run report; anything else is rendered as indented logs. A final
`init.sh __shine_finalize` call performs shared profile integration exactly once.

## Update check (`shine update` / background check)

`cli/src/update_check.rs`:

1. Reads `~/.shine/` cache file; if fresh (24 h TTL, `UPDATE_CACHE_TTL`), no network call.
2. Honors a **rate-limit cooldown**: when GitHub returns a rate-limit error, the
   `rate_limited_until_unix_secs` timestamp (per auth mode) is cached and later checks short-circuit
   until it passes.
3. Otherwise fetches the latest release from GitHub and stores it in the cache.
4. Version-check failures are tolerated in `main.rs` — a failed check must never break the
   primary command the user actually ran.

`shine self upgrade --channel preview` targets the moving `preview` tag instead of the latest
stable `v*` release.

## Config discovery

`config/discovery.rs` priority chain (highest first):
`SHINE_CONFIG_DIR` env → `SHINE_PRESETS` env (presets dir only) → `presets_dir` in
`config.toml` → default `~/.shine/`. Project-local configs inherit unset keys from the global
config (see lessons entry 2026-07-04 on inheritance). `Config` saves go through
`utils::sync_table`, which preserves TOML comments.

## Environment command runner

`env/workspace.rs::handle_run` optionally loads and merges workspace environment sources, then
adds each repeated `--with KEY[=ALIAS]` value from the active config `[env]`. Explicit values use
the same lookup as `env export` (`KEY_SECRET` decrypted first, then plaintext `KEY`) and override
both workspace values and inherited process variables. Without a discovered or explicit
workspace, at least one `--with` is required. The merged environment is applied only to the
spawned child process, whose exit status is propagated by Shine.
