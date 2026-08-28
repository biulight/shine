# Data Flows

End-to-end flows that span multiple modules and are not visible in any single file. For module
ownership and the per-command routing table, see [`module-map.md`](module-map.md) — this file only
records the cross-module sequences and their gotchas.

## Shell install and uninstall

Shell lifecycle targets are either a category (`utils`) or one command in a category
(`utils/shine-env-export`). Embedded sources and external snapshots remain category-scoped shared
deployment material so a command can consume sibling resources, while launchers and
`shell-manifest.toml` receipts are command-scoped.

Command install filters metadata before transforms and launcher creation, then upserts only the
selected manifest target. Category install retains the existing replace-category reconciliation.
Status treats a manifest receipt or a compatible legacy launcher as installed; extracted source
files alone are only cache state. Command uninstall removes only the selected managed launcher,
rendered output, and receipt, rebuilds source-command profile wrappers from the remaining launchers,
and removes shared category material only after the last installed command is gone. Foreign command
entries are never removed.

Every mutating or dry-run Shell lifecycle entry loads `shell-manifest.toml` before extraction,
snapshot, render, launcher, receipt, or profile work. Legacy v0 normalizes in memory, successful
mutations save schema v1, read-only status/update does not rewrite it, and a future version fails
before mutation. The Shell adapter emits one `shell/<category>/<command>` outcome per installed or
selected command. Read-only update maps typed row changes to `pending` plus write-preview effects;
foreign launcher ownership is `conflict`, not pending, and upgrade preserves that launcher and its
receipt. Shared cache/snapshot/rendered effects attach to affected commands without turning source
presence into installation evidence.

Shell execution emits CLI-private presentation events instead of writing terminal output. The
terminal renderer owns shared upgrade-section state, while writer-backed recording tests pin
quiet/verbose sections, conflicts, profile hints, and stdout/stderr routing.

## App install (`shine app install <category>`)

Phase 2 Core extraction owns App manifest/schema types, transforms, hashes, persistence, and normal
managed-file effects in `shine-core`. `CoreRuntime` also provides a host-neutral prepared App
executor used by the Core-only harness; it loads the manifest before mutation, maps Contract v1,
and preserves target isolation. CLI compatibility adapters continue to render the established
events while remaining orchestration is migrated behind that executor.

`cli/src/apps/mod.rs` orchestrates:

1. **Metadata** — `apps/metadata.rs` parses `presets/app/<category>/shine.toml` (category `dest`,
   optional per-`[[files]]` `dest`, `transforms`, `requires_admin`, …). A file destination overrides
   the category root. Exact `macos`/`linux`/`windows` selection (with `unix` as the macOS/Linux
   fallback) filters a targeted category before env loading or embedded extraction; an exact
   destination overrides the fallback. `{ base = "data-dir", path = "..." }` remains structured
   until a `Config` resolves the current user's platform data directory. Duplicate effective
   destinations fail before any writes. Legacy categories without `shine.toml` (git, starship) use
   `apps/annotation.rs` to read a `shine-dest:` comment from the file itself.
2. **Runtime state gate** — `install_core/manifest.rs` loads `app-manifest.toml` before env
   initialization, embedded extraction, generator execution, or destination writes. A missing
   `schema_version` is legacy v0 and normalizes to v1 in memory; an unsupported future version
   fails before lifecycle mutation.
3. **Transforms** — `install_core/transforms/` applies `jsonc-to-json` and/or `template`
   (`@@VAR@@` substitution from the `[env]` config table) in declaration order, in memory, before
   writing.
4. **File ops** — `install_core/file_ops.rs` backs up any pre-existing user file to
   `<name>.shine.bak`, then writes the (transformed) content to `dest`. Destinations with
   `requires_admin = true` (e.g. `/etc/docker/daemon.json`) go through the sudo path, serialized
   by a cross-process advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as mutex).
5. **Manifest** — `install_core/manifest.rs` upserts an `AppEntry` into `~/.shine/app-manifest.toml`,
   recording dest, content hash, strategy, and **`requires_admin`** (must persist — uninstall
   routes on it; see lessons entry 2026-07-04). Successful saves from mutation commands write
   `schema_version = 1`.
6. **Result and report** — the App adapter records safe file/receipt canonical targets, logical
   resources, statuses, effects, and diagnostic codes in `shine-core`'s `LifecycleResultV1`.
   App upgrade, hooks, implicit teardown, embedded preset-cache, and purge join the same result;
   hook and teardown failures retain their non-fatal command semantics. CLI-private presentation
   events flow through a writer-backed reporter, and stale cleanup confirmation uses the frontend
   interaction adapter. Reusable results never include absolute destinations, content, raw errors
   or child output, environment values, or secret values.

## App uninstall

Reverse of install, driven entirely by the manifest — never by re-scanning presets:

1. Look up the `AppEntry` by dest in `~/.shine/app-manifest.toml`.
2. Remove the installed file (sudo path if the entry has `requires_admin = true`).
3. Restore `<name>.shine.bak` if one exists.
4. Remove the manifest entry.

## App update (`shine update`)

App update loads active categories, the App manifest, and effective env once, then derives each
`AppRow` and `LifecycleOutcomeV1` from the same `AppFileAssessment`. This keeps terminal filtering
compatible while preventing an automatic generator from running a second time solely to build the
structured result. Manifest-owned current files are `unchanged`; source, new-file, relocation, or
missing-destination work is `pending`; user-modified destinations are `conflict` with a safe code
and preservation effect. Preview effects describe resource/receipt work without copying the
assessment's absolute paths or content into the reusable result.

## App upgrade (`shine upgrade`)

`apps/upgrade.rs::handle_upgrade_installed` re-applies presets (including re-running transforms
with the *current* `[env]` values) to every manifest-tracked install, and cleans up stale entries
whose preset no longer exists. `shine upgrade app/<category>` selects manifest entries before the
stale/update loop, so no other app category can be mutated. Shell and managed-sys targeted upgrades
apply the same pre-mutation filtering at their own category/item boundaries. `env/upgrade.rs` does
the same for env-templated content. This is why changing an env var requires `shine upgrade` to
take effect in installed files.

Manifest identity for app files is the preset `source`, while ownership checks remain destination-
based. If metadata changes a source's effective destination, upgrade installs the new copy and
removes/restores the old one only after verifying the old content is still manifest-current and the
new destination is free. A modified old copy or occupied new destination blocks relocation without
creating a duplicate manifest entry.

Managed sys resources participate in the same flow. `shine update` compares the desired built-in
resource receipt derived from the active env against `sys-manifest.toml`; `shine upgrade` then
re-applies recorded managed resources and replaces the receipt after convergence. For split DNS,
the receipt comparison includes the normalized domain, DNS servers, and platform resource path.
Update and sys-info output render those receipt differences field by field (`old -> new`) so the
user can inspect the pending system change before granting administrator access to upgrade.
The managed-file driver compares only its desired destination and content hash with the recorded
receipt and emits safe field labels rather than paths or content.

Managed apply/upgrade/uninstall loads the independently versioned `sys-manifest.toml` before
resource, elevation, or composed-profile mutation. Read-only update uses the same typed receipt
comparison to produce both the existing field-difference rows and `pending` `sys/<item>` outcomes.
Built-in drivers return typed resource/backup effects and typed user-modification conflicts; the
adapter never classifies reusable results by parsing `detail` or raw errors. No-op upgrade does not
rewrite the receipt merely to refresh metadata. Bootstrap execution, profile enable/disable, and
composed-profile sync remain outside the structured managed-resource result.

Managed Sys presentation also flows through the CLI reporter. Item ownership is rendered before
the interaction adapter requests administrator authorization, preserving prompt context without
making terminal or privilege APIs part of the reusable lifecycle result.

`shine update --diff` expands stale shell/app rows, while `shine update <TARGET>` resolves one
installed shell/app through the same aliases as `shine info` and prints only its stale files. Each
row carries structured pending changes: content, source/destination relocation, a new file,
deployment metadata, or command-entry refresh. Only content changes invoke `info`'s effective-
content renderer; structural changes are rendered field by field. Inline diffs require valid UTF-8
without NUL bytes and are capped at 256 KiB per side. Embedded versus external preset selection,
transforms, and manual-generator behavior stay identical to `shine info --diff` and the upgrade
operation. Target mode returns after the config check and does not perform the binary release check;
managed sys resources keep their structured receipt differences instead.

## Generated app files

An app `[[files]]` entry may declare
`generator = { script, runtime, env, when_env, auto }`. The static `source` remains a
safe fallback and stable manifest identity. When `when_env` exists in the active
`[env]` table, `apps::materialize_file_content` runs the generator and uses its
UTF-8 stdout as the effective source before normal transforms and install
strategies:

1. `shine app install` always materializes first, then reuses the normal
   manifest hash and atomic file installer.
2. `auto` defaults to true; automatic generators retain the existing behavior
   of materializing during status/update checks and `shine upgrade`.
3. `auto = false` makes status local-only and excludes the file from upgrade.
   `shine app refresh <category> [source]` explicitly refreshes only
   manifest-owned generated files, with an optional `--force` for user changes.
4. An existing managed destination is the last-known-good snapshot when a
   generator fails; a first-time enabled generator failure is fatal.
5. Only `generator.env` values are injected. External preset or overlay
   generator code requires `allow_app_hooks = true`. Execution is deadline- and
   output-size-limited.
6. A Bun generator is resolved against the physical category that supplied its effective script.
   Embedded temporary scripts use `--no-install`; an external/overlay script uses
   `--install=fallback` only with a valid `package.json` + `bun.lock` pair in that category.

The Surge generator downloads the Base64 URI list in
`SURGE_SUBSCRIPTION_URL`, converts supported SS/VMess nodes, and writes bare
policy declarations to `subscription-proxies.conf`. It declares `auto = false`
so it runs on install (including `--replace-managed`) or explicit refresh, not ordinary
status/upgrade passes. Its `Subscription` group
loads that file through `policy-path`; other groups reuse the nodes through
`include-other-group=Subscription`. VLESS and unsupported transports are
counted and skipped without logging credentials.

## App artifact build (`shine app artifact apply <app-id>`)

`apps/build.rs::handle_build` is fully separate from install/upgrade — it never runs
automatically; see [ADR 0009](../decisions/0009-app-artifact-build-explicit-command.md). Given an
app preset's `[artifact].script`:

1. Resolves the category the same way `app info`/`app install` do
   (`metadata::load_active_categories`), force-extracting embedded assets first when not in
   external-presets mode (the same `extract_prefix` call `app install` makes).
2. Resolves the script's location: an overlay's `app/<name>/<script>` wins when that exact script
   exists; otherwise the source (built-in or external) script is used. This lets an overlay keep
   local policy files while inheriting a generic built-in artifact.
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
6. For Bun artifacts and teardown, the final script source selects the dependency policy: embedded
   or unlocked external code uses `--no-install`; a locked external/overlay category uses
   `--install=fallback`. Resolution does not alter the cwd, environment contract or permission gate.

For Surge specifically, `shine app install surge` copies the local files and
the generated-subscription fallback into the Surge Profiles dir. The built-in
Bun `build.ts` atomically appends `local-proxies.conf`,
`local-proxy-groups.conf`, and `local-rules.conf` to the corresponding section
includes after `SURGE_PROFILE` is configured. It preserves permissions and line
endings, rejects symlink profiles, and fails when an expected section has no
patchable include. Subscription nodes are not added to `[Proxy]`;
`local-proxy-groups.conf` loads the generated bare policy file through
`policy-path`.

**Teardown (`shine app artifact remove <app-id>`, ADR 0012).** An `[artifact].teardown` script reverses
`build`, sharing the *identical* resolution and env contract above (steps 1–4). It has two entry
points: `handle_unbuild` (explicit, ungated, errors propagate — symmetric to `build`) and
`run_teardown_for_uninstall`, called best-effort from `apps/uninstall.rs` *before* the file-removal
loop (implicit, so gated by `allow_app_hooks` for external presets and non-fatal, and a no-op under
`--dry-run`). Surge ships a symmetric built-in Bun `unbuild.ts`; other app
presets may still keep artifact-specific reversal logic in an overlay.

**Lifecycle command hooks (`apps/hooks.rs`).** `post_install` (fired by `install`, including
`--replace-managed`) and
`post_upgrade` (fired by `upgrade`) share one runner, `run_app_hooks(config, get_category, changed,
HookPhase)` — run once per *changed* category, gated by `allow_app_hooks` for external presets,
failures non-fatal. These are plain argv commands with only the inherited parent env — distinct from
the richer `SHINE_APP_*` + `[env]` artifact contract used by `build`/`teardown`.

## Shell install / uninstall

The Shell source/deployment model, canonical target parser, external mode, and versioned manifest
are Core-owned. The CLI deployment module consumes those types while retaining the current
distribution adapter for embedded `rust-embed` assets and terminal presentation.

Embedded install extracts assets, links executables into `~/.shine/bin/` (`bin_links.rs`), and
appends a sentinel-guarded PATH block to the shell config (`shells/profile.rs`). Uninstall removes
only Shine-managed symlinks/files and deletes the sentinel block precisely.

For external presets, `external_shell_mode = "snapshot"` first materializes the effective
base/overlay category under `<shine_dir>/installed/shell/`; update compares that snapshot with the
active source and upgrade refreshes it. Explicit `live` mode points raw commands at the external
category. Materialization skips every `node_modules/` directory but preserves `package.json` and
`bun.lock`.

Every Bun launcher includes an explicit package policy. Embedded commands and unlocked external
commands use `--no-install`. When the physical category owning an effective external/overlay script
contains both lock files, the launcher uses `--install=fallback`; the Shell manifest records this
mode and a combined content hash. Snapshot changes are applied by upgrade. Live execution reads the
current package files immediately, while status reports that its receipt and launcher need refresh.
A transformed live launcher calls the manifest-constrained internal renderer on each
invocation, then executes or sources the atomically refreshed file under `rendered/`.

## Workspace environment export

`shine env workspace export --format dotenv` resolves one explicit mode through the same ordered
source paths as `env run`, but deliberately excludes inherited process variables and `--with`
injection. The default path parses only `[plain]` values and does not decrypt payloads; a later
secret declaration removes any earlier plain value with the same key. `--include-secrets` switches
to the normal sealed-source compiler, then writes the standalone plaintext result through an atomic
owner-only file on Unix. Export never edits or removes the workspace definition or source files.

## Sys bootstrap (`shine sys bootstrap`)

Sys driver/status types, receipts, resource outcomes, and `sys-manifest.toml` are Core-owned.
`CoreRuntime` has an in-memory managed-file apply/remove executor that shares App file ownership
primitives; platform-specific split-DNS/bootstrap/profile orchestration remains on the active Phase
2 migration path and does not expand Lifecycle Contract v1.

Selection resolves explicit ordered items, a named selection profile, or the existing
interactive/default path through `sys/selection.rs`. Explicit items accept only `mode = "init"`,
deduplicate by first occurrence, and never widen to sibling items.

Every executable sys manifest declares `version = 2`. Each init item has both `[items.detect]` and
`[items.install]`; Rust performs the read-only detection, invokes a fixed Homebrew/APT/Winget
provider argv or one per-item script, limits runtime/output, detects again, and produces the
canonical `sys/<item>` outcome. A v1 or unknown manifest version fails before detection, elevation,
installer execution, or profile writes.

Successful bootstrap items set `profile_enabled` in `sys-manifest.toml`. `sys/profile_compose.rs` combines base pre/post content
with all enabled item integrations in stable manifest order. `sys/profile.rs` reconciles the two
generated files before `sys/profile_blocks.rs` updates the existing pre/post sentinels. Composition
happens once after item execution, and render failure leaves the last installed profile intact.
`sys profile enable/disable` changes only this activation state and generated profile content.

Shine does not run update checks for bootstrap software. Homebrew, APT, Winget, mise, rustup, or
the applicable upstream tool owns package versions and upgrades. Global `shine update` / `shine
upgrade` remain limited to Shine configuration and managed sys resources.

Top-level `shine list` reads current-OS entries with `managed = true` directly from
`sys-manifest.toml` for its installed-only `System Configs` section. It does not call the live
desired-state checker; `shine update` remains responsible for showing only pending managed
changes.

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
usable. `shine preset overlay link --git <url>` writes the config and clones immediately;
`configured_targets` deliberately excludes the managed dir from the fast-forward path. See
[ADR 0010](../decisions/0010-git-managed-overlay.md).

## Config discovery

`config/discovery.rs` priority chain (highest first):
`SHINE_CONFIG_DIR` env → `SHINE_PRESETS` env (presets dir only) → `presets_dir` in
`config.toml` → default `~/.shine/`. Project-local configs inherit unset keys from the global
config (see lessons entry 2026-07-04 on inheritance). `Config` saves go through
`utils::sync_table`, which preserves TOML comments.

## Dynamic shell completion

`main.rs` calls `completion::complete_from_env` before Clap parsing, Tokio startup, config
initialization, schema warnings, or update checks. Registration and each Tab request build the
Clap command graph and attach dynamic candidates for active preset resources, recorded sys items,
and saved tasks. Candidate callbacks use `config::discover_runtime_paths_read_only`, which mirrors
global/project preset and overlay inheritance synchronously without creating `~/.shine`, then read
only the small preset metadata or runtime manifest needed for the active argument. Parse or I/O
failures are tolerated and never break the user's shell.

## Environment command runner

`env/workspace.rs::handle_run` optionally loads and merges workspace environment sources, then
adds each repeated `--with KEY[=ALIAS]` value from the active config `[env]`. Explicit values use
the same lookup as `env secret export` (`KEY_SECRET` decrypted first, then plaintext `KEY`) and override
both workspace values and inherited process variables. Without a discovered or explicit
workspace, at least one `--with` is required. The merged environment is applied only to the
spawned child process, whose exit status is propagated by Shine.

## Transparent environment proxies

`shine env proxy install <command> --with KEY` places a Shine-owned PATH shim
ahead of the real CLI. The shim records the resolved real executable and invokes
`env::proxy::exec`, which reloads the effective global/project configuration,
selects that command's `[[env_proxy]]` rule, and injects only its declared
values (`KEY_SECRET` decrypted first, then `KEY`) into the child process.
Project rules replace the global rule for the same command. The shim never
exports values to the parent shell and never scans all `_SECRET` values.
Each rule defaults to `enabled = true`; `shine env proxy disable <command>`
retains the shim but bypasses config lookup and secret decryption entirely.

## SSH environment forwarding

`ssh::handle_ssh` resolves each `--with KEY[=ALIAS]` from the exact plaintext key in the active
config `[env]`; it never performs the secret-first fallback used by `env run`. Each
`--with-secret KEY[=ALIAS]` instead loads `KEY_SECRET` and decrypts it through the tag-routed
secret backend. Duplicate aliases and Shine's own SSH/session variable names are rejected. The
default `--remote-shell posix` flow joins the resolved map, `SHINE_SSH_*`, and terminal-theme hint
in the quoted `env ... sh -c` wrapper and creates the transfer listener/`-R` channel. The explicit
`--remote-shell windows` flow instead encodes a PowerShell script as UTF-16LE Base64, probes for
`pwsh.exe` (PowerShell 7), and falls back to `powershell.exe` (Windows PowerShell 5.1); it sets only
the session hint, theme, and selected variables and creates no listener, reverse forward, or
`shine local` channel. Its interactive child loads the selected PowerShell's normal profile so
managed PATH entries and source-command wrappers are available; an explicit remote command keeps
`-NoProfile` for deterministic execution. Values are session-only but
necessarily exposed in process argv/environments on the local and remote hosts; see
[ADR 0014](../decisions/0014-explicit-ssh-env-forwarding.md) and
[ADR 0015](../decisions/0015-windows-ssh-environment-forwarding.md).

## SSH on-demand secret broker

`shine ssh --secret-broker` attaches a local `ssh::broker::BrokerSession` to the existing reverse
control channel. The local process freezes the active config, encrypted direct-secret inputs, and
the policy file for the lifetime of that SSH session; private-key operations remain local.

For a direct remote request, `shine env run --no-workspace --secret-broker --secret KEY[=ALIAS]
-- <argv>` sends only the requested mapping and argv. The local agent checks the SSH session's
`--allow-secret` list, pauses the interactive SSH child, restores the local TTY for an explicit
confirmation, decrypts only `KEY_SECRET`, and returns the selected values. Direct requests always
confirm, including in a trusted workspace session.

For a workspace request, `env/workspace.rs` reads the workspace and all selected source files once
into a bounded `WorkspaceSnapshot`. The request carries the exact bytes and SHA-256 identities,
mode, complete declared-secret set, requested release mapping, and argv. `env/broker.rs` accepts
only an exact match in `<shine_dir>/ssh-secret-broker.toml`; the local agent then confirms unless
`--trust-remote-session` was explicitly set, decrypts only the policy-approved release subset, and
sends values back. The remote merges those values with non-secret entries parsed from the same
snapshot bytes and injects them only into the child process.

`shine ssh --secret-broker-inspect` displays one remote description without decrypting or writing.
`--secret-broker-enroll --trust-remote-metadata` may create a local policy after local confirmation
when the operator explicitly trusts the remote. The safer normal path is
`shine env broker policy add`, generated from a trusted local workspace checkout. See
[ADR 0024](../decisions/0024-ssh-on-demand-secret-broker.md) and the
[secret-broker PRD](../../ssh-secret-broker-prd.md).

Policy describe/add/update/diff accept either explicit repeated `--release KEY` or
`--release-all-declared`. The latter expands the current snapshot into a sorted explicit release
array; no wildcard reaches disk or the wire. Any new declared secret changes the source identity
and fails closed until policy update. When no trusted local checkout exists,
`--secret-broker-enroll --trust-remote-metadata --update-policy NAME` previews a full diff and may
replace exactly one same-mode/same-argv allow while preserving the named policy's local identity
fields; a concurrent local policy edit aborts the write.

## Personal task runner (`shine task run` / `shine run`)

`task::handle_run` loads `<shine_dir>/tasks.toml` (`task::manifest::TaskManifest`), looks up the
named task's saved argv, appends any `-- EXTRA...` args, and spawns it with
`std::process::Command` — **directly, with no shell** — inheriting the caller's stdio and
environment. When the task has an explicit `cwd`, saved as a canonical absolute path by
`task save --cwd`, the handler validates it and sets `Command::current_dir`; missing `cwd` retains
the caller's current directory for backward compatibility. The child's exit code is propagated verbatim (`std::process::exit(code)`; on Unix a
terminating signal becomes `128 + signal`), never wrapped in an anyhow error, so the task's own
exit semantics survive Shine in the middle. `shine run <NAME>` is a top-level alias routed to the
same handler. `task::handle_save` validates the name (`[A-Za-z0-9._-]`, letter/digit start) and
rejects an empty command or a duplicate without `--force`; `info`/`list` render the argv back to a
copy-paste-safe line by shell-quoting shell-significant arguments.

## Secret backend routing (GPG / age)

Every call site that decrypts a stored secret (`env secret decrypt`, `env secret export`, workspace
`env secret seal`/`env run`) goes through `secret::decrypt_secret(ciphertext, age_identities)`, which inspects the
ciphertext for an `age:` prefix (`secret::parse_tagged_ciphertext`) and dispatches to
`secret::age`/`secret::gpg` accordingly; untagged ciphertext is always GPG. Decryption never
reads `Config::secret_backend` — only the tag decides. Encryption (`env secret encrypt`, workspace
`env secret seal`) instead resolves a `secret::EncryptRecipients` (CLI `-r`/`--backend` > workspace
`env.encryption` > `config.toml` `gpg_recipients`/`age_recipients`/`secret_backend` > GPG default)
and calls `secret::encrypt_secret`, which tags age output and leaves GPG output untagged. See
[ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md) for the full rationale.
`shine env secret identity init [--touch-id]` generates the age identity file
(`age-keygen`/`age-plugin-se keygen`) consulted via `Config::age_identities()`.
