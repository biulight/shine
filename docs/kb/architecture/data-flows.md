# Data Flows

End-to-end flows that span multiple modules and are not visible in any single file. For the
module map and per-command routing table, see [`AGENTS.md`](../../../AGENTS.md) — this file only
records the cross-module sequences and their gotchas.

## App install (`shine app install <category>`)

`cli/src/apps/mod.rs` orchestrates:

1. **Metadata** — `apps/metadata.rs` parses `presets/app/<category>/shine.toml` (`dest`,
   `transforms`, `requires_admin`, …). Legacy categories without `shine.toml` (git, starship)
   use `apps/annotation.rs` to read a `shine-dest:` comment from the file itself.
2. **Transforms** — `install_core/transforms/` applies `jsonc-to-json` and/or `template`
   (`@@VAR@@` substitution from the `[env]` config table) in declaration order, in memory, before
   writing.
3. **File ops** — `install_core/file_ops.rs` backs up any pre-existing user file to
   `<name>.shine.bak`, then writes the (transformed) content to `dest`. Destinations with
   `requires_admin = true` (e.g. `/etc/docker/daemon.json`) go through the sudo path, serialized
   by a cross-process advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as mutex).
4. **Manifest** — `install_core/manifest.rs` upserts an `AppEntry` into `~/.shine/app-manifest.toml`,
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

Managed sys resources participate in the same flow. `shine update` compares the desired built-in
resource receipt derived from the active env against `sys-manifest.toml`; `shine upgrade` then
re-applies recorded managed resources and replaces the receipt after convergence. For split DNS,
the receipt comparison includes the normalized domain, DNS servers, and platform resource path.
Update and sys-info output render those receipt differences field by field (`old -> new`) so the
user can inspect the pending system change before granting administrator access to upgrade.

## App artifact build (`shine app build <app-id>`)

`apps/build.rs::handle_build` is fully separate from install/upgrade — it never runs
automatically; see [ADR 0009](../decisions/0009-app-artifact-build-explicit-command.md). Given an
app preset's `[artifact].script`:

1. Resolves the category the same way `app info`/`app install` do
   (`metadata::load_active_categories`), force-extracting embedded assets first when not in
   external-presets mode (the same `extract_prefix` call `app install` makes).
2. Resolves the script's location: the overlay's `app/<name>/<script>` wins over the source
   (built-in or external) copy *if the overlay's copy of that category exists at all* — decided
   once for the whole category directory, not per file (unlike `Config::preset_path`'s per-file
   overlay precedence used for installed content), since a build script's sibling files (e.g. a
   `provider-url` file the script reads directly) are one package with the script.
3. Creates (idempotently, before spawning) `SHINE_APP_HTTP_DIR` (`<shine_dir>/http/app/<name>`),
   `SHINE_STATE_DIR` (`<shine_dir>/state/app/<name>`), and `SHINE_CACHE_DIR` (the OS cache dir via
   the `directories` crate — `<os-cache>/shine/app/<name>`, the same crate/pattern
   `env/workspace.rs::cache_path` already uses for its own per-project cache).
4. Injects the active `[env]` table into the child **as stored** (`EnvConfig::as_map` — no
   decryption, same as the `template` transform), so a script can read user-configured values such
   as `SURGE_PROFILE`; `_SECRET` keys pass through as ciphertext and no build ever triggers a
   secret-decryption prompt. The `SHINE_APP_*` contract vars are set *after* the `[env]` values, so
   they win on any name collision.
5. Runs the script with `current_dir` set to the resolved app directory and inherited stdio (not
   captured like `post_upgrade` hooks), so build output streams live; a nonzero exit becomes a
   real `Result::Err` instead of being swallowed.

For Surge specifically: `shine app install surge` is a plain `Copy` install of `local-proxies.conf`
/ `local-rules.conf` into the Surge Profiles dir (`dest`), and `shine app build surge` runs the
overlay `build.sh`, which reads `$SURGE_PROFILE` and appends `, local-proxies.conf` /
`, local-rules.conf` to the `[Proxy]` / `[Rule]` `#!include` lines of the user's active profile.
Surge itself owns the subscription (`#!MANAGED-CONFIG`); shine no longer fetches or serves it.

**Teardown (`shine app unbuild <app-id>`, ADR 0012).** An `[artifact].teardown` script reverses
`build`, sharing the *identical* resolution and env contract above (steps 1–4). It has two entry
points: `handle_unbuild` (explicit, ungated, errors propagate — symmetric to `build`) and
`run_teardown_for_uninstall`, called best-effort from `apps/uninstall.rs` *before* the file-removal
loop (implicit, so gated by `allow_app_hooks` for external presets and non-fatal, and a no-op under
`--dry-run`). Reversal logic stays in the overlay's `unbuild.sh`; shine core never learns what the
patch was.

**Lifecycle command hooks (`apps/hooks.rs`).** `post_install` (fired by `install`/`reinstall`) and
`post_upgrade` (fired by `upgrade`) share one runner, `run_app_hooks(config, get_category, changed,
HookPhase)` — run once per *changed* category, gated by `allow_app_hooks` for external presets,
failures non-fatal. These are plain argv commands with only the inherited parent env — distinct from
the richer `SHINE_APP_*` + `[env]` artifact contract used by `build`/`teardown`.

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

`cli/src/update_check/` (`mod.rs` core + cache, `github.rs` API/auth, `upgrade.rs` install flow):

1. Reads `~/.shine/` cache file; if fresh (24 h TTL, `UPDATE_CACHE_TTL`), no network call.
2. Honors a **rate-limit cooldown**: when GitHub returns a rate-limit error, the
   `rate_limited_until_unix_secs` timestamp (per auth mode) is cached and later checks short-circuit
   until it passes.
3. Otherwise fetches the latest release from GitHub and stores it in the cache.
4. Version-check failures are tolerated in `update_check::maybe_notify` (called from `main.rs`) —
   a failed check must never break the primary command the user actually ran.

`shine self upgrade --channel preview` targets the moving `preview` tag instead of the latest
stable `v*` release.

## Git-managed preset pull

`git_pull.rs::handle_pull` resolves the effective `presets_dir` and any *manually linked* overlay
(`presets_overlay_dir`) to their Git roots, de-duplicates shared repositories, and validates every
worktree before running `git pull --ff-only`. Dirty worktrees, detached HEADs, missing upstreams,
and pull failures stop the operation. `update --pull` and `upgrade --pull` pull first, reload
`Config`, then check or apply presets so updated project and environment configuration takes effect
immediately. Successful pulls are summarized as one line per repository (commit range plus short
file stats), while raw Git progress is hidden unless the parent update/upgrade command is verbose.
Failed pulls always include captured Git diagnostics; non-Git and duplicate sources are only shown
verbosely.

A **shine-managed Git overlay** (`presets_overlay_git`) is handled separately, *before* the
fast-forward loop, by `git_pull::sync_managed_overlay` against `<shine_dir>/overlay`. On first use
it clones `--depth 1` via a temp sibling dir + atomic rename (a failed clone never leaves a
half-populated overlay). On subsequent runs it **force-mirrors**: `git fetch --depth 1 origin
<branch>` then `git reset --hard FETCH_HEAD`, so the checkout always equals the remote tip even
across rebases/force-pushes, discarding local edits (the managed overlay is read-only by design).
The fetch runs before the reset, so an unreachable remote leaves the previous checkout intact and
usable. `shine overlay link --git <url>` writes the config and clones immediately;
`configured_targets` deliberately excludes the managed dir from the fast-forward path. See
[ADR 0010](../decisions/0010-git-managed-overlay.md).

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

## Personal task runner (`shine task run` / `shine run`)

`task::handle_run` loads `<shine_dir>/tasks.toml` (`task::manifest::TaskManifest`), looks up the
named task's saved argv, appends any `-- EXTRA...` args, and spawns it with
`std::process::Command` — **directly, with no shell** — inheriting the caller's stdio and
environment. The child's exit code is propagated verbatim (`std::process::exit(code)`; on Unix a
terminating signal becomes `128 + signal`), never wrapped in an anyhow error, so the task's own
exit semantics survive Shine in the middle. `shine run <NAME>` is a top-level alias routed to the
same handler. `task::handle_save` validates the name (`[A-Za-z0-9._-]`, letter/digit start) and
rejects an empty command or a duplicate without `--force`; `info`/`list` render the argv back to a
copy-paste-safe line by shell-quoting shell-significant arguments.

## Secret backend routing (GPG / age)

Every call site that decrypts a stored secret (`env decrypt`, `env export`, workspace
`seal`/`run`) goes through `secret::decrypt_secret(ciphertext, age_identities)`, which inspects the
ciphertext for an `age:` prefix (`secret::parse_tagged_ciphertext`) and dispatches to
`secret::age`/`secret::gpg` accordingly; untagged ciphertext is always GPG. Decryption never
reads `Config::secret_backend` — only the tag decides. Encryption (`env encrypt`, workspace
`seal`) instead resolves a `secret::EncryptRecipients` (CLI `-r`/`--backend` > workspace
`env.encryption` > `config.toml` `gpg_key_id`/`age_recipients`/`secret_backend` > GPG default)
and calls `secret::encrypt_secret`, which tags age output and leaves GPG output untagged. See
[ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md) for the full rationale.
`shine env identity init [--touch-id]` generates the age identity file
(`age-keygen`/`age-plugin-se keygen`) consulted via `Config::age_identities()`.
