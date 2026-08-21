# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

`shine` is a self-contained Rust CLI that bundles shell scripts, app config presets, and OS
bootstrap presets into one binary (rust-embed), installs them under `~/.shine/`, and supports
safe, manifest-tracked uninstall. The workspace root is the publishable `shine-cli` package
(binary + `cli` library sources under `cli/`); `utils/` is the reusable `shine-core` package.

## Where knowledge lives

| Need | Read |
|---|---|
| Public user manual (English default + Simplified Chinese) | [`docs/manual/`](docs/manual/), [`website/i18n/zh-Hans/`](website/i18n/zh-Hans/) |
| Build/test/lint commands, module map, command routing, preset authoring | this file |
| Cross-module data flows | [`docs/kb/architecture/data-flows.md`](docs/kb/architecture/data-flows.md) |
| Invariants that must not be broken | [`docs/kb/architecture/invariants.md`](docs/kb/architecture/invariants.md) |
| Why things are the way they are (ADRs) | [`docs/kb/decisions/`](docs/kb/decisions/) |
| Commit/versioning/testing conventions | [`docs/kb/conventions.md`](docs/kb/conventions.md) |
| Release runbook, CI pipelines, troubleshooting | [`docs/kb/operations/`](docs/kb/operations/) |
| Past bugs and the rules they taught | [`docs/kb/lessons.md`](docs/kb/lessons.md) |

Start any non-trivial task by checking `docs/kb/architecture/invariants.md` and grepping
`docs/kb/lessons.md` for the modules you are about to touch.

Keep the KB alive by updating it **in the same change** that makes it stale: bug with a
non-obvious cause → `lessons.md`; design choice → numbered ADR in `decisions/`; changed data
flow or invariant → the matching file under `architecture/`; moved/renamed modules → sync this
file. Full protocol: [`docs/kb/README.md`](docs/kb/README.md).

User-visible behavior changes must update the matching English and Simplified Chinese manual pages
in the same release change. English is the default locale under `docs/manual/`; Simplified Chinese
lives under `website/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`. Keep doc IDs and page sets
aligned, preserve commands and identifiers verbatim, and do not publish `docs/kb/`, PRDs, or release
runbooks through the public site. The root READMEs are summaries and must not grow back into a second
complete command or configuration reference.

## Commands

```bash
# Toolchain setup (versions pinned in mise.toml)
mise install
bun install --frozen-lockfile

# Build
cargo build
cargo build --release          # binary at target/release/shine

# Run (dev)
cargo run -- shell list
cargo run -- shell install
cargo run -- shell install proxy
cargo run -- shell uninstall --dry-run
cargo run -- sys list
cargo run -- sys bootstrap --dry-run
cargo run -- env list
cargo run -- self upgrade --channel preview

# Test (pre-commit uses nextest)
cargo nextest run --all-features
cargo test                     # fallback without nextest
bun run test:ts                # Bun preset tests

# Single test
cargo test shells::tests::install_then_uninstall_roundtrip
cargo nextest run -E 'test(install_then_uninstall)'

# Lint / format
cargo fmt
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo deny check bans licenses sources
typos                          # spell-check
bun run typecheck              # strict TypeScript check
bun run check:ts               # type-check + Bun tests

# Public documentation
cd website
pnpm install --frozen-lockfile
pnpm check:locales
pnpm typecheck
pnpm build
```

Rust and Bun versions are pinned in `mise.toml`; run `mise install` once and activate mise in your
shell before using the commands above. Then run `bun install --frozen-lockfile` before editing the
Bun TypeScript presets. Pre-commit validates `mise.toml` and runs
`mise exec -- bun run check:ts` when Bun tooling or TypeScript sources change, alongside
`cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check`, `typos`, and
`cargo nextest run`. All must pass before committing.

## Verification Notes

- In sandboxed environments, prefer `cargo ... --target-dir target` for ad hoc builds/tests/runs. This keeps build artifacts in the repo-local ignored `target/` directory and avoids permission failures from a global Cargo target dir.
- Most `shine` commands call `Config::load_or_init()` and may create config state even for read-oriented commands. Use a repo-local ignored config dir when verifying CLI behavior:

```bash
mkdir -p .tmp-home/.shine
env SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine cargo run --target-dir target -- app list
```

- `SHINE_CONFIG_DIR` has higher priority than `SHINE_PRESETS` and `config.toml` `presets_dir`. When it is set, the runtime presets directory is `$SHINE_CONFIG_DIR/presets/`.
- Built-in app listing commands read embedded presets unless an external presets mode is active:
  - Without `SHINE_CONFIG_DIR`, `SHINE_PRESETS`, or `presets_dir`, use `cargo run --target-dir target -- app list` and `cargo run --target-dir target -- app info <category>` to verify embedded app metadata.
  - With `SHINE_CONFIG_DIR` set, `app list` / `app info` verify presets from `$SHINE_CONFIG_DIR/presets/app/`; copy the preset under test there first, or unset `SHINE_CONFIG_DIR`.
- `app install <category> --dry-run` uses the runtime presets directory when external presets mode is active. With `SHINE_CONFIG_DIR` set, copy the preset under test into `.tmp-home/.shine/presets/app/<category>/` before running install dry-runs.
- For metadata-driven app presets, verify with:
  - a targeted unit test for destination resolution or metadata parsing
  - `cargo run --target-dir target -- app list`
  - `cargo run --target-dir target -- app info <category>`
  - `cargo run --target-dir target -- app install <category> --dry-run`

## Architecture

### Workspace layout

```
shine/
├── Cargo.toml     # shine-cli package root and workspace manifest
├── cli/           # Main binary sources ("shine"), backed by a lib crate ("cli")
│   ├── build.rs  # cargo:rerun-if-changed=presets (rust-embed trigger)
│   └── src/
│       ├── lib.rs            # Module tree root for the `cli` library crate
│       ├── main.rs           # Bin crate root: `fn main`, `run()` dispatch; delegates
│       │                     # `init` to `cli::init::handle_init` and the background
│       │                     # version check to `cli::update_check::maybe_notify`
│       ├── init.rs           # `shine init`: confirm + write a project-local
│       │                     # shine.config.toml. `pub mod` in lib.rs, same
│       │                     # lib-testability reasoning as shim.rs.
│       ├── shim.rs           # Top-level install/uninstall canonical TARGET routing;
│       │                     # unique bare categories are shorthand, ambiguity is an error.
│       │                     # `pub mod` in lib.rs (not bin-private), so its unit
│       │                     # tests run under `cargo test --lib` too.
│       ├── home.rs           # effective_home_dir (sudo-aware), tilde/full path expansion
│       ├── preset_commands.rs # preset export/copy/link/unlink/pull, overlay link/unlink/info.
│       │                     # `pub mod` in lib.rs, same lib-testability reasoning as shim.rs.
│       ├── self_install.rs   # update/self-upgrade/upgrade-installed-configs,
│       │                     # atomic self-install binary copy. `pub mod` in lib.rs,
│       │                     # same lib-testability reasoning as shim.rs.
│       ├── commands/
│       │   ├── mod.rs        # Clap subcommand enums (ShellCommands, AppCommands, etc.)
│       │   ├── cli.rs        # Cli, Commands, CompletionShell/Commands, and the
│       │   │                 # other top-level clap arg types (lives in the lib
│       │   │                 # crate since completion.rs needs them)
│       │   ├── app.rs        # AppCommands enum
│       │   ├── env.rs        # EnvCommands enum
│       │   ├── preset.rs     # PresetCommands + export/copy/link/overlay arg types
│       │   ├── state.rs      # StateCommands (`state migrate`)
│       │   ├── self_install.rs # SelfCommands enum (install, upgrade)
│       │   ├── shell.rs      # ShellCommands enum
│       │   └── sys.rs        # SysCommands enum
│       ├── apps/
│       │   ├── mod.rs        # Module root: mod declarations + re-exports (handle_*,
│       │   │                 # handle_init_template); shared kernel used by install/uninstall/
│       │   │                 # info/upgrade (resolve_install_destination, source_*_for_file,
│       │   │                 # desired/installed_content_hash, install_prepared_content,
│       │   │                 # uninstall_app_entry, app_category_from_source/app_source_parts)
│       │   ├── install.rs    # handle_install
│       │   ├── uninstall.rs  # handle_uninstall, category-scoped manifest-entry selection
│       │   ├── info.rs       # handle_info, handle_list
│       │   ├── report.rs     # Install/uninstall outcome print_* helpers
│       │   ├── upgrade.rs    # handle_upgrade_installed, stale-entry cleanup
│       │   ├── hooks.rs      # run_app_hooks: shared post_install/post_upgrade command-hook
│       │   │                 # runner (HookPhase), gated by allow_app_hooks for external presets
│       │   ├── generator.rs  # [[files]].generator runner: condition/env resolution,
│       │   │                 # timeout/output limits, external-code permission gate
│       │   ├── refresh.rs    # `shine app refresh`: explicit category/file generator
│       │   │                 # refresh with manifest ownership + user-modification guard
│       │   ├── build.rs      # handle_build/handle_unbuild: run an app preset's [artifact].script
│       │   │                 # (`shine app artifact apply`) / [artifact].teardown (`shine app artifact remove`),
│       │   │                 # never run implicitly by install/upgrade. run_teardown_for_uninstall
│       │   │                 # runs teardown best-effort on uninstall — see ADR 0009 + 0012
│       │   ├── metadata.rs   # shine.toml manifest parsing (AppCategory, AppFile, AppArtifact)
│       │   ├── json_merge.rs # JsonMerge install strategy (managed-key merge)
│       │   └── annotation.rs # shine-dest: comment annotation parser
│       ├── install_core/     # Install primitives shared by apps/ and sys/ (no
│       │   │                 # apps-specific logic; sys depends on this, not on apps)
│       │   ├── mod.rs        # Re-exports: AppEntry, AppInstallStrategy, AppManifest,
│       │   │                 # hash_content, apply_transforms
│       │   ├── file_ops.rs   # File copy, backup (*.shine.bak), restore, admin_lock
│       │   ├── manifest.rs   # ~/.shine/app-manifest.toml tracking (AppManifest, AppEntry)
│       │   └── transforms/   # File content transforms: jsonc-to-json, template
│       ├── env/
│       │   ├── mod.rs        # EnvConfig: [env] table in config.toml, @@VAR@@ substitution
│       │   ├── broker.rs     # SSH secret-broker workspace snapshots, local policy store,
│       │   │                 # exact policy matching, inspect/trusted enrollment
│       │   ├── commands.rs   # env list/set/delete/get + `env secret` handlers
│       │   ├── catalog.rs    # Known env-var metadata (description, sensitive) for `env list`
│       │   ├── identity.rs   # `shine env secret identity init/list`: age identity generation
│       │   │                 # (age-keygen / age-plugin-se --touch-id) and recipient inspection
│       │   ├── upgrade.rs    # Re-apply env template transforms to installed presets
│       │   └── workspace.rs  # `env secret seal` / `env run`: workspace env files, `--with` injection
│       ├── git_pull.rs       # Safe FF-only pulls for Git-managed preset sources
│       ├── shells/
│       │   ├── mod.rs        # Module root: ShellType, SENTINEL_*, get_shell/get_shell_config_path,
│       │   │                 # mod declarations + re-exports (handle_*, ShellUpgradeReport)
│       │   ├── deployment.rs # External snapshot/live deployment, shell-manifest.toml,
│       │   │                 # category materialization, constrained lazy live transforms
│       │   ├── install.rs    # category or category/command handle_install,
│       │                     # handle_upgrade_installed/handle_completion_install/
│       │   │                 # handle_init_template, script/link-spec building
│       │   ├── uninstall.rs  # category or category/command handle_uninstall
│       │   ├── links.rs      # Bin-symlink spec building, link-conflict detail/printing
│       │   ├── report.rs     # handle_list, ShellUpgradeReport, install/uninstall summary formatting
│       │   ├── profile.rs    # Managed profile file/PATH/sentinel-block install+removal
│       │   ├── template.rs   # @@VAR@@ template rendering for installed scripts
│       │   └── metadata.rs   # ShellCategory/ShellFile parsing from shine.toml or .sh files
│       ├── sys/
│       │   ├── mod.rs        # Module root: mod declarations + re-exports (handle_*, detect_os_id)
│       │   ├── commands.rs   # handle_list/handle_info/handle_status/handle_init orchestration,
│       │   │                 # preset-manifest loading
│       │   ├── bootstrap.rs  # standard read-only detection + fixed Homebrew/APT/Winget
│       │   │                 # ensure-present providers and per-item script executor
│       │   ├── managed.rs    # Managed-item command family: SysAction, handle_apply/
│       │   │                 # handle_uninstall/handle_upgrade_managed, managed_updates,
│       │   │                 # run_managed (converge/remove a managed sys resource)
│       │   ├── render.rs     # Presentation helpers: sys_init_theme, print_available_item,
│       │   │                 # item/driver name labels, print_dry_run
│       │   ├── detect.rs     # detect_os_id / detect_os_id_from (OS + Linux distro detection)
│       │   ├── model.rs      # SysManifest/SysItem/SysItemStatus/SysItemOutcome/SelectionSource, etc.
│       │   ├── run_manifest.rs # SysRunManifest/SysRunEntry: ~/.shine/sys-manifest.toml load/save
│       │   ├── manifest.rs   # Preset loading, parsing, and validation
│       │   ├── profile.rs    # Sys-profile install: loader install, three-way merge
│       │   │                 # (fallback + git merge-file), conflict markers
│       │   ├── profile_compose.rs # Deterministic base + enabled item integration composer
│       │   ├── profile_commands.rs # `sys profile enable/disable` activation handlers
│       │   ├── profile_blocks.rs # Shell-profile sentinel blocks: per-phase sentinel
│       │   │                 # insert/remove, BOM preservation, legacy-sentinel migration
│       │   ├── selection.rs  # Item-selection resolution (profile vs interactive)
│       │   ├── execution.rs  # Bootstrap reporting, proxy environment, and outcome formatting
│       │   ├── resources.rs  # SystemDriver trait, receipts, BuiltinDriver glue (dispatches
│       │   │                 # to sys/drivers/*)
│       │   └── drivers/
│       │       ├── mod.rs         # pub(super) mod declarations for the driver submodules
│       │       ├── split_dns.rs   # Split-DNS driver: desired-state, apply/remove, Windows NRPT
│       │       └── managed_file.rs # Managed-file driver: desired-state, apply/remove
│       ├── config/
│       │   ├── mod.rs        # Config struct + accessors, Default, new_for_test
│       │   ├── load.rs       # load_or_init, global/project layering, schema version read
│       │   ├── save.rs       # Atomic save, comment-preserving merge, sparse project diff
│       │   ├── env_layer.rs  # [env] defaults/parsing, removed env.toml guard, override files
│       │   └── discovery.rs  # Project-config discovery, SHINE_CONFIG_DIR/
│       │                     # SHINE_PRESETS priority chain
│       ├── presets.rs        # rust-embed asset extraction, list_categories, parse_script_description
│       ├── bin_links.rs      # ~/.shine/bin/ command management: LinkRuntime (Native symlink/shim
│       │                     # vs Bun launcher), marker-based launcher ownership + current-ness
│       ├── status.rs         # Shared install-status row builders used by `list`/`info`
│       ├── state.rs          # `shine state migrate`: versioned runtime-state cleanup
│       ├── colors.rs         # Terminal color helpers
│       ├── serve.rs          # Local HTTP server for shine-managed resources under ~/.shine/http/
│       ├── list.rs           # Top-level `shine list` and status views
│       ├── path_display.rs   # Home-relative path formatting for terminal output
│       ├── proc.rs           # Small subprocess helpers with no domain logic (ensure_command)
│       ├── sentinel.rs       # Shared sentinel-block find/extract/remove/insert primitives used
│       │                     # by shells/profile.rs and sys/profile_blocks.rs
│       ├── secret/
│       │   ├── mod.rs        # BackendKind/EncryptRecipients, tagged-ciphertext router
│       │   │                 # (encrypt_secret/decrypt_secret); untagged = gpg, `age:` = age
│       │   ├── exec.rs       # Shared subprocess helpers (TempFile, base64 encode/decode) used
│       │   │                 # by both backends below
│       │   ├── gpg.rs        # GPG-backed encrypt/decrypt, untagged base64 ciphertext
│       │   └── age.rs        # age-backed encrypt/decrypt, multi-recipient + Secure Enclave
│       │                     # (age-plugin-se) identity support, `age:`-tagged ciphertext
│       ├── info/
│       │   ├── mod.rs        # `shine info <TARGET>` orchestration and update diffs
│       │   ├── collect.rs    # Gathers installed AppInfoFile/ShellInfoFile data from manifests
│       │   ├── resolve.rs    # Resolves a TARGET string to an InfoRef via canonical/alias matching
│       │   └── render.rs     # println!/diff formatting for app and shell info output
│       ├── ssh/
│       │   ├── mod.rs        # `shine ssh`: wraps system ssh, arg splitting, session
│       │   │                 # bootstrap, wrapped remote command (env vars + EXIT trap)
│       │   ├── broker.rs     # Local-only broker session: authorization, TTY confirmation,
│       │   │                 # replay protection, decrypt-on-demand
│       │   ├── protocol.rs   # Wire format (control + log relay): Transfer request,
│       │   │                 # Starting/Log/Done frames. Shared by agent.rs and remote_client.rs
│       │   ├── agent.rs      # Local transfer server: spawns rsync/scp (build_transfer_argv,
│       │   │                 # choose_tool) and relays their output. Unix socket or
│       │   │                 # loopback TCP (Windows) via LocalListener
│       │   ├── session_context.rs # SessionContext (host/ssh_options/local_dir/control_path)
│       │   │                 # captured at `shine ssh` time; source of the rsync/scp reconnect args
│       │   └── remote_client.rs # `shine local download/upload` handlers (run on remote host)
│       ├── task/
│       │   ├── mod.rs        # `shine task` save/run/list/info/delete handlers,
│       │   │                 # optional fixed cwd, direct (no-shell) argv exec + exit-code passthrough,
│       │   │                 # shell-quoted command rendering, task-name validation
│       │   └── manifest.rs   # TaskManifest: <shine_dir>/tasks.toml load/save/upsert
│       ├── theme/
│       │   ├── mod.rs        # `shine theme sync`: priority chain (already-exported
│       │   │                 # SHINE_TERMINAL_THEME -> COLORFGBG -> OSC 11), BAT_THEME
│       │   │                 # resolution/preservation, read-only config load, also exposes
│       │   │                 # resolve_local_terminal_theme_for_injection for `shine ssh`
│       │   ├── color.rs      # Pure parsing: OSC 11 rgb: body, luma light/dark threshold,
│       │   │                 # COLORFGBG — no I/O, cross-platform
│       │   └── osc.rs        # #[cfg(unix)]: OSC 11 query over /dev/tty. Deadline-based
│       │                     # poll(2) read loop (total deadline, never per-byte — see
│       │                     # docs/kb/lessons.md 2026-07-14) + termios EchoGuard (RAII
│       │                     # restore, unlike the superseded shell script's manual restore)
│       ├── test_support.rs   # Shared test-only env-var mutex (not cfg(test)-gated,
│       │                     # since #[cfg(test)] doesn't cross the lib/bin boundary)
│       ├── update_check/
│       │   ├── mod.rs        # ReleaseChannel/UpdateStatus/UpgradeResult, check_for_update(_forced),
│       │   │                 # `maybe_notify` (gates the background check main.rs runs per-command,
│       │   │                 # never fails the user's command on check failure), 24h disk cache
│       │   ├── github.rs     # GitHub API types, release/asset fetch, auth-token resolution,
│       │   │                 # rate-limit-aware error formatting
│       │   └── upgrade.rs    # `upgrade_to_release`: asset selection, archive download/extract,
│       │                     # staged-swap binary install with rollback on failure
│       └── version.rs        # Version string formatting
├── utils/        # shine-core library crate: shared helpers with no CLI/Tauri dependencies
│   └── src/
│       ├── migration.rs      # TOML comment-preserving sync (utils::sync_table)
│       └── init_template.rs  # write_shine_toml_template (shared by `preset new app|shell`)
└── presets/      # Embedded assets (compiled into binary via rust-embed)
    ├── shell/
    │   ├── agent/   cc.ts, cc.test.ts, shine.toml  (cross-platform Bun `ccenv`; launches Claude with a selected provider)
    │   ├── proxy/   set_proxy.sh, uset_proxy.sh, shine.toml
    │   └── utils/   copyfile.sh, shine.toml
    ├── app/
    │   ├── archey4/    config.json, shine.toml
    │   ├── clash-verge/ merge.yaml (inert commented composite EXAMPLE with file/loopback/HTTPS rule-provider alternatives — real overlay copy is hardcoded, no templating), rules/*.list inert Option 1 references (per-file `data-dir` dest installs them under CVR mihomo HomeDir `ruleset/shine-source/`; HTTP/HTTPS modes ignore them), build.ts + build.test.ts + unbuild.ts (bun), shine.toml  (category dest = ~/.shine/clash-verge; plain Copy. `shine app artifact apply clash-verge` reads profiles.yaml to resolve the current subscription's merge/rules/proxies/groups bindings; renders rule-providers to Merge and proxies/proxy-groups/prepend-rules into the three CVR 2.x `{ prepend, append, delete }` editor files; never falls back to global files or mutates profiles.yaml/bindings/cache. A changed write asks the user to reselect the profile; a later build refreshes providers through CLASH_CONTROLLER_URL/TOKEN. See docs/clash-verge-local-subscription-prd.md)
    │   ├── docker-desktop/ settings-store.jsonc, shine.toml
    │   ├── docker-engine/  daemon.jsonc, shine.toml
    │   ├── fastfetch/  config.jsonc, shine.toml
    │   ├── ghostty/    config.ghostty, shine.toml
    │   ├── git/        gitconfig  (shine-dest: ~/.gitconfig; no shine.toml, uses annotation instead)
    │   ├── JetBrains/  shine.toml
    │   ├── surge/      local-proxies.conf, local-proxy-groups.conf, local-rules.conf, rules/*.list examples, subscription-proxies.conf + generate-subscription.ts/tests (Bun Base64 SS/VMess generator; VLESS skipped), build.ts + unbuild.ts + profile-artifact.ts/tests (Bun; atomic profile section-include patch/teardown), shine.toml  (dest = Surge Profiles dir; Subscription group loads generated policies through policy-path)
    │   ├── starship/   starship.toml  (shine-dest: ~/.config/starship.toml; no shine.toml, uses annotation instead)
    │   └── vim/        shine.toml, vimrc, _machine_specific.vim
    └── sys/
        ├── macos/   shine.toml, install/, profile/
        ├── ubuntu/  shine.toml, install/, profile/
        └── windows/ shine.toml, profile/
```

### Command routing

`main.rs` → `Commands` enum → module handlers:

| Top-level command | Handler module |
|---|---|
| `shell list/info/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `install/uninstall <TARGET>` | `cli/src/shim.rs` → `apps/` or `shells/` |
| `list [--available [KIND]]` | `cli/src/list.rs` or scoped list handlers |
| `app artifact apply/remove <app-id>` | `cli/src/apps/build.rs` |
| `app refresh <app-id> [file]` | `cli/src/apps/refresh.rs` |
| `sys list/bootstrap` | `cli/src/sys/` |
| `theme sync` | `cli/src/theme/` (bypasses `Config::load_or_init()`, like `init`/`state migrate`) |
| `env list/set/get/delete/run` / `env secret ...` / `env broker ...` | `cli/src/env/` |
| `info <TARGET>` | `cli/src/info/` (installed or available app/shell; `sys/` for explicit system items) |
| `preset export/copy/link/unlink/overlay` | `cli/src/preset_commands.rs` |
| `preset pull` / `update --pull` / `upgrade --pull` | `cli/src/git_pull.rs` + `main.rs` routing |
| `init` | `cli/src/init.rs` |
| `self install/upgrade` | `cli/src/self_install.rs` + `update_check/` |
| `update [TARGET]` / `upgrade [TARGET]` | `cli/src/self_install.rs` + filtered app/shell/sys upgrade handlers + `update_check/` |
| `state migrate` | `cli/src/state.rs` |
| `serve install/start/status/uninstall/url` | `cli/src/serve.rs` |
| `completions` | `main.rs` inline (clap_complete) |
| `ssh [--with ...] [--with-secret ...] [--secret-broker ...] [SSH_ARGS]... <HOST> [COMMAND]` | `cli/src/ssh/mod.rs` |
| `local download/upload/status` | `cli/src/ssh/remote_client.rs` |
| `task save/run/list/info/delete` | `cli/src/task/` |
| `run <NAME>` (alias for `task run`) | `cli/src/task/` |

### Key data flow

**Install** (`shine shell install [CATEGORY[/COMMAND]]`):
1. Embedded mode uses `presets::extract_prefix`; external snapshot mode materializes the
   effective category under `<shine_dir>/installed/shell/`; explicit live mode retains the
   external source path. Category sources are shared deployment material even for a command target.
2. `bin_links::link_executables(bin_dir, sources)` creates flat entries in `~/.shine/bin/` only for
   the selected commands; `shell-manifest.toml` records the same command-scoped activation.
3. `shells::append_path_to_shell_config` — appends a sentinel-guarded `export PATH` block to `~/.zshrc` (or equivalent)

**Uninstall**:
1. `bin_links::unlink_managed` removes a category, while command targets remove only their selected
   managed entry and manifest receipt; foreign entries are never touched.
2. `presets::remove_prefix` removes only embedded-asset files after the last installed command no
   longer needs the shared category material (user files are never touched).
3. `shells::remove_path_from_shell_config` — removes the sentinel block; skipped on `--dry-run`

### Env variable substitution

`shine env set KEY VALUE` writes to the `[env]` table in `config.toml`. App (and shell) preset files that declare `transforms = ["template"]` in their `shine.toml` have `@@KEY@@` placeholders replaced at install/upgrade time. Run `shine upgrade` after changing env vars to re-apply to installed presets.

`shine env secret encrypt`/`shine env secret decrypt` store secrets as base64 ciphertext in the `[env]` table,
routed through `secret::encrypt_secret`/`decrypt_secret` (`secret/mod.rs`) to either the GPG
(`secret/gpg.rs`, untagged ciphertext) or age (`secret/age.rs`, `age:`-tagged ciphertext, with
optional Apple Touch ID via `age-plugin-se`) backend. Decryption always routes purely on the
ciphertext's tag, never on config — see
[ADR 0008](docs/kb/decisions/0008-age-secret-backend-tagged-ciphertext.md). `shine env secret identity
init/list` (`env/identity.rs`) manages the age identity file consulted via
`Config::age_identities()`.

### File transforms (`install_core/transforms/`)

Two transforms can be applied to preset files at install time (declared in `shine.toml` as `transforms = [...]`):

- **`jsonc-to-json`** — strips JSONC-style comments so pure-JSON apps can consume the output (used by `docker-engine/daemon.jsonc`, `docker-desktop/settings-store.jsonc`, `fastfetch/config.jsonc`).
- **`template`** — substitutes `@@VAR_NAME@@` placeholders from the active `[env]` config table.

Transforms compose in declaration order: `transforms = ["jsonc-to-json", "template"]`.

The `template` delimiter is `@@VAR@@` for **every** file type — there is deliberately no
per-file-type delimiter. `@` is a YAML reserved indicator only as the first char of a plain
scalar, so YAML presets either follow the clash-verge overlay pattern (hardcode real values, no
templating) or quote the placeholder (`key: "@@VAR@@"`, which yields a string). Native-typed env
rendering into YAML is not supported today; if ever needed it would be an explicit opt-in
`template_open`/`template_close`, not extension inference. See
[ADR 0013](docs/kb/decisions/0013-template-delimiter-policy.md).

### SSH session transfer flow (`shine ssh` / `shine local`)

See [`docs/ssh-local-transfer-prd.md`](docs/ssh-local-transfer-prd.md) for the full design and
[ADR 0011](docs/kb/decisions/0011-ssh-local-transfer-rsync-scp.md) for the current transport.
The tunnel carries a **control + log-relay** channel: the remote sends one `Transfer` request and
the local agent runs `rsync` (default) / `scp` and streams its output back. Windows support (local
side only — see step 8) rides on top of the macOS/Linux implementation.

1. `shine ssh [SSH_ARGS]... <HOST> [COMMAND]` generates a session id + token
   and spawns `ssh` with `-t -R <remote-sock>:<local-forward-target>`
   prepended to the user's own args (`ssh::split_ssh_args` locates the
   destination/command boundary without reinterpreting ssh's own option
   semantics). `<remote-sock>` is always a Unix socket path under `/tmp` on
   the remote host (assumed Linux/macOS); `<local-forward-target>` is
   platform-dependent — see step 8. Unless the user already set their own
   multiplexing, shine also injects `-o ControlMaster=auto -o
   ControlPath=<session_dir>/ctl.sock -o ControlPersist=60` so the later
   rsync/scp child reconnects over this authenticated master with no second
   auth prompt (ADR 0011). The captured `host`/`ssh_options`/cwd/`control_path`
   are stored as `ssh::session_context::SessionContext` (in-memory `Arc`, also
   written to `<session_dir>/context.toml`) — the sole, local-trusted source of
   the rsync/scp reconnect args.
2. The remote command is replaced with a wrapper that sets
   `SHINE_SSH_SESSION`/`SHINE_SSH_TOKEN`/`SHINE_SSH_REMOTE_SOCK` via `env`
   (not `SetEnv`/`SendEnv`, which most `sshd_config`s reject), then `exec`s
   the user's original remote command or their login shell. Repeated `--with
   KEY[=ALIAS]` entries add exact plaintext values from the active local `[env]`;
   `--with-secret KEY[=ALIAS]` is the separate explicit opt-in that decrypts
   `KEY_SECRET`. Values are shell-quoted into the same wrapper and last only for
   the session; see ADR 0014.
3. sshd does **not** clean up the forwarded remote socket file on
   disconnect (verified against a real host via `scripts/spike-ssh-forward.sh`
   before implementation) — the wrapper registers its own `trap ... EXIT`.
4. `shine local download/upload` (run on the remote host) reads those env
   vars and sends one `ClientMessage::Transfer { direction, remote_spec,
   local_spec, force, dry_run, use_scp }` over the forwarded socket. The
   remote-owned spec is absolutized against the remote cwd
   (`remote_client::absolutize_remote_spec`) with glob metacharacters
   preserved (string join, never canonicalized), so `download '<dir>/*.log'`
   is expanded by rsync/scp's remote shell for free. The only authorization is
   the session token, which travels to the remote as plain argv/environ and is
   readable by other local users there via `ps eww`, so wire fields
   (`remote_spec`/`local_spec`) are **untrusted** — see step 5.
5. The local agent (`ssh::agent`) validates the token, resolves the local side
   against the session directory (tilde-only expansion via `home::tilde_expand`,
   never `${VAR}`; upload-source globs expanded with the `glob` crate), picks the
   tool (`choose_tool`: rsync by default, scp on `--scp` or as an auto-fallback
   with a printed notice when rsync is missing locally or — probed over the
   control master — on the remote), and spawns it via `build_transfer_argv`.
   **Security (ADR 0011, `docs/kb/lessons.md`):** argv only, never a shell; the
   remote path is emitted only as the single token `<host>:<remote_spec>` after
   a `--` separator (so a hostile `-oProxyCommand=…` becomes an inert
   `host:-…`); local operands are anchored to the session dir and `./`-prefixed
   if dash-leading; the `-e`/`-o` reconnect string comes solely from
   `SessionContext`, never the wire. rsync directories/symlinks/perms are
   handled natively; no-`--force` maps to rsync `--ignore-existing` (scp can't
   gate overwrite, so it warns instead).
6. The child's stdout/stderr are read in bounded chunks and relayed verbatim as
   `ServerMessage::Log { stream, chunk }` frames (preserving `\r` progress
   redraws), followed by `Done { code }`; `remote_client::relay_until_done`
   prints the chunks and propagates the child's exit code as its own. rsync's
   own `--info=progress2` provides progress. `--dry-run` uses rsync `-n`; scp
   has no dry-run, so the agent synthesizes a preview `Log` line and never
   spawns.
7. `shine local status` sends a `Status` request over the same forwarded
   socket; the agent replies with the session's local working directory and
   `host`, and the client also reports the session id (from `SHINE_SSH_SESSION`)
   and negotiated protocol version. If the agent is unreachable, it reports that
   instead of erroring, so the command doubles as a liveness check without
   needing a live session.
8. Windows support is local-side only (the remote host is always assumed
   Linux/macOS; steps 1-3 above never change). `tokio::net::UnixListener`
   doesn't exist on non-unix targets, so `ssh::bind_local_listener` is
   `#[cfg(unix)]`/`#[cfg(windows)]`-gated: unix binds a Unix socket as
   before, Windows binds a loopback TCP listener (`127.0.0.1:0`, OS-picked
   port) and the `-R` argument becomes a *mixed* forward
   (`<remote-unix-sock>:127.0.0.1:<port>`) — verified against a real
   Windows OpenSSH client (`OpenSSH_for_Windows_9.5p2`) via
   `scripts/spike-ssh-forward-windows.ps1`. `agent::LocalListener` is an
   enum over `Unix`/`Tcp` variants (the `Unix` variant itself is
   `#[cfg(unix)]`-gated); `agent::DuplexStream` is a blanket-impl marker
   trait (`AsyncRead + AsyncWrite + Unpin + Send + 'static`) so the
   per-connection protocol logic (`handle_connection`, `handle_transfer`)
   stays transport-agnostic and unchanged for both platforms.
9. `ssh::remote_client` (the *remote*-side of a session — it dials the
   forwarded socket via `UnixStream`, so it only makes sense on
   Linux/macOS, per step 8's scoping) is itself `#[cfg(unix)]`-gated;
   `ssh::handle_local_download`/`handle_local_upload`/`handle_local_status`
   have `#[cfg(not(unix))]` stub implementations that return a clear
   "Windows is local-side only" error, so the binary still compiles for
   Windows. Missing this the first time around broke the real
   `build-preview-assets` Windows CI job
   (`error[E0432]: unresolved import tokio::net::UnixStream` in
   `remote_client.rs`) — this repo's sandboxed dev environment cannot
   fully verify Windows builds (an unrelated transitive C dependency,
   `aws-lc-sys` via `reqwest`, needs the real MSVC toolchain even for
   `cargo check --target x86_64-pc-windows-msvc`), so a cross-check that
   gets past `cli`/`utils` compilation and only fails in `aws-lc-sys`'s
   own build script is the strongest confirmation available without a
   real Windows CI run.
10. `handle_ssh` races the spawned `ssh` child (`cmd.status()`) against
    `tokio::signal::ctrl_c()` so a local Ctrl-C doesn't kill the process
    before cleanup runs — installing the listener overrides SIGINT's
    default disposition, and the `ssh` child (same foreground process
    group) still receives and handles its own SIGINT independently. Each
    accepted connection is handled on its own task tracked in a shared
    `agent::ConnectionTasks` (a `JoinSet`), not a bare detached
    `tokio::spawn` — `agent_handle.abort()` only ever stops the *accept
    loop*, so before removing the session directory (or exiting on a
    nonzero `ssh` status via `std::process::exit`, which skips Rust's
    unwind/drop machinery entirely), `handle_ssh` calls
    `agent::drain_connection_tasks` to wait, up to a bounded grace period,
    for any still-running transfer to notice the now-closed tunnel and
    finish its own cleanup rather than being abandoned mid-copy.

### Sys preset flow (`shine sys bootstrap`)

1. `sys::detect_os_id()` — reads `std::env::consts::OS`; on Linux reads `ID=` from `/etc/os-release`.
2. `presets::extract_prefix("sys/<os_id>", presets_dir)` materializes the detected OS preset.
3. `sys::load_sys_preset` validates the v2 manifest, named selection profiles, detection/install
   metadata, and item-owned shell integrations.
4. `sys::selection` resolves ordered positional items, a named profile, or interactive/default
   selection. Positional items and `--preset` are mutually exclusive; managed items are rejected.
5. `sys::bootstrap` performs standard read-only detection and fixed Homebrew/APT/Winget install argv
   (or one per-item script). Every init item declares both `detect` and `install`; there is no
   platform dispatcher fallback.
6. Successful items enable their integration state. `sys::profile_compose` renders base + enabled
   item integrations once, and `sys::profile` reconciles them through the existing pre/post sentinels.

`--dry-run` prints provider/script actions and persistent integration details without executing.
`--proxy` injects the standard HTTP proxy env set and passes `--proxy` explicitly to Winget. External
or overlay install scripts and executable profile code require `allow_sys_code = true`. Static
detection, provider declarations, PATH, env, and aliases do not. See ADR 0028.

Ubuntu ships three profiles (`presets/sys/ubuntu/shine.toml`): `recommended` (default,
full interactive dev setup), `all` (recommended + zerotier/pnpm/mise/homebrew), and
`minimal` — a lean headless CLI core (`neovim`, `fzf`, `bat`, `eza`, `zoxide`) intended for
production-server bootstrapping via `shine sys bootstrap --preset minimal`. The `minimal` profile
reuses existing items only, so adding it needed no installer change.

### Personal tasks (`shine task` / `shine run`)

`shine task` is a lightweight personal shortcut-command registry, kept separate
from the preset/install machinery: it is **runtime/user state**, not an embedded
preset. Tasks are stored in `<shine_dir>/tasks.toml` as an argv array per name:

```toml
[tasks.deploy-keystone]
command = ["rsync", "-avz", "dist/", "marqueeio.develop:/var/www/keystone/alex/"]
```

- Because tasks live under `Config::shine_dir()`, the store follows
  `SHINE_CONFIG_DIR` automatically (test isolation needs no extra plumbing).
- `shine task run <NAME> [-- EXTRA...]` executes the saved argv **directly with
  no shell** (`std::process::Command`), inheriting the caller's stdio/env, and
  propagates the child's exit code verbatim (never wrapped in an anyhow error).
  Extra args after `--` are appended to the saved argv.
- `shine run <NAME>` is a top-level alias for `shine task run <NAME>` with no
  independent semantics or storage.
- `shine task info`/`list`/`run` render the saved argv back to a copy-paste-safe
  command line by shell-quoting arguments that contain shell-significant
  characters.
- **Platform limit:** direct execution runs any real executable on every
  platform, but the `sh -c '...'` escape hatch for pipes/redirects is Unix-only
  (Windows has no `sh`).

### Config (`~/.shine/config.toml`)

`Config::load_or_init()` resolves directories with this priority:
1. `SHINE_CONFIG_DIR` env var — overrides both shine dir and presets dir
2. `SHINE_PRESETS` env var — overrides presets dir only
3. `presets_dir` key in `config.toml`
4. Default: `~/.shine/` (shine dir), `~/.shine/presets/` (presets dir)

Config is saved via `utils::sync_table` which preserves existing TOML comments while updating values.

### Presets overlay

An overlay merges over the active presets source by matching relative paths (`Config::preset_path`,
`presets::read_asset_bytes`/`asset_paths`, `apps/build.rs`). There are two mutually exclusive ways to
configure it, both resolved by `Config::active_presets_overlay_dir()`:

- **Manual** — `presets_overlay_dir` in `config.toml` points at a user-owned directory
  (`shine preset overlay link <path>`). Fast-forward-pulled by `shine preset pull` like any Git preset source.
- **shine-managed Git** — `presets_overlay_git` (+ optional `presets_overlay_git_branch`) records a
  Git URL (`shine preset overlay link --git <url>`). shine owns the checkout at `<shine_dir>/overlay`
  (`managed_overlay_dir`, resolved by `Config::resolve_managed_overlay_dir` from `shine_dir`, so it
  follows `SHINE_CONFIG_DIR`). It is cloned `--depth 1` on first `shine preset pull` and **force-mirrored**
  (`git fetch --depth 1` + `reset --hard`) to the remote tip afterward — a read-only mirror, never
  fast-forward-pulled. See `git_pull::sync_managed_overlay`. A managed overlay is only "active" once
  its checkout exists on disk; setting one via the CLI clears any manual `presets_overlay_dir`.

### Local HTTP resources

`shine serve start` serves files from `~/.shine/http/` on a single loopback HTTP server
(`127.0.0.1:6174` by default). On macOS, `shine serve install` registers that same server as one
user launchd service; it is intentionally global, with no per-app argument. App presets that need
stable local URLs should install files under that tree (for example
`~/.shine/http/app/<name>/<file>`) and use `shine serve url <path>` to print the URL. Do not start
one HTTP service per app preset. (The `surge` preset no longer uses this — it installs local files
into the Surge Profiles dir and patches the profile's `#!include` lines instead.)

### rust-embed and presets

`PresetAssets` (in `presets.rs`) embeds everything under the workspace-root `presets/` directory at compile time. `build.rs` registers `cargo:rerun-if-changed=presets` so cargo recompiles when preset files change — without this, new/modified scripts won't appear in the binary.

### Shell config PATH injection

`append_path_to_shell_config` writes a sentinel block to the detected shell config file:
```
# >>> shine >>>
if [[ ":$PATH:" != *":$HOME/.shine/bin:"* ]]; then
  export PATH="$HOME/.shine/bin:$PATH"
fi
# <<< shine <<<
```
`bin_dir` paths under `home_dir` are expressed as `$HOME/...` for portability. `remove_path_from_shell_config` deletes the block precisely, including the preceding blank line separator.

On Windows, PowerShell profile updates target both `~/Documents/PowerShell/Microsoft.PowerShell_profile.ps1` and `~/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1` so `pwsh.exe` and Windows PowerShell stay in sync.

### `shine shell list`

Reads embedded assets, groups them by immediate subdirectory under `shell/`, and displays per-script descriptions. If a category has `shine.toml`, the metadata file drives file listing and command names. Without `shine.toml`, descriptions are parsed from the leading comment block of each `.sh` file (lines starting with `# ` after the shebang, until the first non-comment line).

Shell categories can declare `needs_source = true` in `shine.toml` to mark a script as requiring `source` (not direct execution). These are exposed as shell functions rather than symlinked commands. Entries can also declare `platforms = ["unix"]` or `platforms = ["windows"]` to ship different source files for the same command name. Cross-platform helpers such as `ccenv` should prefer one `runtime = "bun"` entry when they do not need to modify the parent shell.

## Git Push Policy

**Never `git push` to the remote without explicit user approval.** Commit locally, then stop and let the user review before pushing. This applies to branch pushes, tag pushes, and force-pushes.

## Releases

Hard rules (details and runbook: [`docs/kb/conventions.md`](docs/kb/conventions.md),
[`docs/kb/operations/release-runbook.md`](docs/kb/operations/release-runbook.md),
[ADR 0002](docs/kb/decisions/0002-hand-written-changelog.md)):

- `CHANGELOG.md` is **hand-written** — never generate it with `git cliff`. (The `git cliff` run
  in `release.yml` produces the GitHub Release notes body, a separate automated artifact.)
- Version-bump baseline is the latest **stable `v*` tag**, never the moving `preview` tag:
  `git tag --list 'v*' --sort=-version:refname | head -1`.
- Internal-only fix commits caused by new code in the same release must use the
  git-cliff-skipped scopes (`fix(lint|clippy|fmt|typo|build|ci|internal)`) — full table in
  `docs/kb/conventions.md` § Commits.
- Work lands on the `release` branch; `main` only receives automated post-release sync PRs
  ([ADR 0001](docs/kb/decisions/0001-release-branch-model.md)).

## Adding a new preset category

### Shell preset category

1. Create `presets/shell/<category>/your_script.sh` with a `#!/bin/bash` shebang and a multi-line `# description` comment block immediately after it.
2. Optionally add `presets/shell/<category>/shine.toml` to control command names, enable `needs_source`, or set a category description.
   ```toml
   description = "What this category does."

   [[files]]
   source = "your_script.sh"
   target = "mycommand"        # symlink/function name exposed in PATH
   needs_source = false        # set true for scripts that must be sourced
   ```
3. `cargo build` will re-embed automatically (tracked by `build.rs`).
4. `shine shell list` will display the new category; `shine shell install <category>` will install it.

Bun entries: a `[[files]]` entry may set `runtime = "bun"` with a `.ts`/`.js`/`.mts`/`.mjs`
`source` to expose a cross-platform command run via `bun <script> "$@"`. Bun is an explicit
external prerequisite — shine never installs it and only *checks* for it (a missing `bun` makes the
generated launcher exit `127` with an install hint). Rules: `runtime = "bun"` cannot combine with
`needs_source = true` (a subprocess can't mutate the parent shell — keep the thin `.sh`/`.ps1`
wrapper for env mutation); the source must be a bun extension; only `[[files]]`-listed entries
become commands (helper `.ts` modules are ignored by auto-collection). Env templating for bun (and
any) entries is opt-in via `transforms = ["template"]` (the `# shine-template: true` annotation is
`.sh`/`.ps1`-only, since `#` is not a JS/TS comment). Unlike native commands (Unix symlink / Windows
shim), a bun command installs a **shine-managed regular launcher file** — see the launcher ownership
invariants in [`architecture/invariants.md`](docs/kb/architecture/invariants.md) and the PRD
[`docs/bun-shell-presets-prd.md`](docs/bun-shell-presets-prd.md).

Describe a bun entry with a `//` comment header at the top of the source (the JS/TS mirror of the
`.sh`/`.ps1` `#` block, parsed by `presets::parse_bun_description`), or set `description = "…"` in
the `[[files]]` entry (works for any runtime, mirrors app presets) — an explicit `description` wins
over the header. `shine shell list` shows the full block; `shine info` shows its first line.

A bun `[[files]]` entry may also declare `env = ["KEY", "SOURCE=TARGET"]` (same grammar as `shine
env run --with`, ordered, duplicate targets rejected, names validated at metadata-load time; valid
only when `runtime = "bun"`). At launch the generated launcher runs the child through `shine env run
--no-workspace --with … -- bun <script>` so the resolved values reach the script via `Bun.env`;
`<KEY>_SECRET` is decrypted per invocation (no cache — a Touch ID / pinentry backend prompts every
run). This adds a runtime dependency on `shine` being on `PATH` (missing `shine` → exit `127`, like
missing `bun`); entries with no `env` keep the v1 `bun <script>` launcher unchanged. `env`
(runtime, no upgrade needed) and `transforms = ["template"]` (static `@@VAR@@`, needs `shine
upgrade`) are independent and may combine. Full design:
[`docs/bun-shell-preset-env-injection-prd.md`](docs/bun-shell-preset-env-injection-prd.md).

### App preset category

Prefer `shine.toml` metadata over legacy `shine-dest:` annotations for new categories. Place `shine.toml` in `presets/app/<category>/` with at minimum `dest = "~/<path>"`. Add `transforms = ["jsonc-to-json"]` for JSONC files or `transforms = ["template"]` for files with `@@VAR_NAME@@` env placeholders.

App categories may declare `post_upgrade` and/or `post_install` hooks (each a `{ command = "...", args = ["..."] }` table or an array of them) to run a direct argv command after a category actually changes. `post_upgrade` fires when `shine upgrade` updates/installs ≥1 file in the category; `post_install` fires when `shine app install` writes ≥1 file (including `--replace-managed`; a plain re-install with no change runs nothing, mirroring `post_upgrade`). Both share one runner (`apps/hooks.rs::run_app_hooks`): external presets require `allow_app_hooks = true` before hooks execute, and each hook may set `show_output = true` to print its stdout on success (defaults to `false`/silent). Hooks inherit only the parent env — no `SHINE_APP_*`/`[env]` injection (that is the artifact contract below). Declare `post_install` when the very first install must run a setup/reload that `post_upgrade` would otherwise only do on a later upgrade.

An app `[[files]]` entry may declare `generator = { script = "...", runtime = "native|bun", env = ["KEY", "SOURCE=TARGET"], when_env = "KEY", auto = false }`. The static `source` is mandatory and is used as the fallback plus stable manifest identity; when `when_env` exists, the generator's UTF-8 stdout becomes the effective source before transforms. `auto` defaults to true, preserving install/update/upgrade materialization. With `auto = false`, implicit status paths stay local-only and upgrade preserves the installed snapshot; install (including `--replace-managed`) still generates, while `shine app refresh <category> [source] [--force]` explicitly refreshes manifest-owned generated files. Only declared config env values are injected (no `_SECRET` decryption); external preset/overlay generators require `allow_app_hooks = true`. Keep stdout deterministic and put only credential-free summaries on stderr. A failed refresh keeps an existing managed file. See [ADR 0016](docs/kb/decisions/0016-generated-app-files-and-surge-subscriptions.md) and [ADR 0018](docs/kb/decisions/0018-manual-app-generator-refresh.md).

App categories may also declare `[artifact]\nscript = "build.sh"` (optionally `teardown = "unbuild.sh"`) to expose `shine app artifact apply <app-id>` / `shine app artifact remove <app-id>` entry points. An artifact may set `runtime = "bun"` (default is `native`) so its script runs via `bun <script>` — cross-platform (macOS/Windows/Linux), like the bun shell presets, and requiring `bun` on PATH; `native` execs the script file directly (relying on its shebang, so Unix-only). A `bun` artifact's `script`/`teardown` must be a `.ts`/`.js`/`.mts`/`.mjs` file. The built-in `clash-verge` and `surge` presets use `runtime = "bun"`. Unlike the hooks above, these never run implicitly from `install`/`upgrade` — `script` runs only on explicit `shine app artifact apply`, and `teardown` runs on explicit `shine app artifact remove` **and** best-effort during `shine app uninstall`. Both scripts receive the fixed `SHINE_APP_*` env contract **plus the active `[env]` table passed as stored** (no decryption — `_SECRET` keys arrive as ciphertext, same as the `template` transform), so scripts can read user-configured values like `SURGE_PROFILE` without triggering a secret-decryption prompt. The explicit artifact commands are not gated by `allow_app_hooks` and propagate a nonzero exit as a real error; the implicit teardown during `uninstall` **is** gated (external presets) and is non-fatal (a broken teardown never blocks file removal). An overlay artifact still takes precedence when the exact script exists. For the built-in `surge` preset, install copies the local files plus the `subscription-proxies.conf` fallback into the Surge Profiles dir; the trusted Bun file generator may replace that fallback from `SURGE_SUBSCRIPTION_URL` during install (including `--replace-managed`) or explicit `app refresh` only. Built-in `build.ts` atomically patches `[Proxy]`/`[Proxy Group]`/`[Rule]` includes, while `local-proxy-groups.conf` loads generated policies via `policy-path`; `unbuild.ts` reverses the section includes. See [ADR 0009](docs/kb/decisions/0009-app-artifact-build-explicit-command.md), [ADR 0012](docs/kb/decisions/0012-app-lifecycle-post-install-and-teardown.md), [ADR 0016](docs/kb/decisions/0016-generated-app-files-and-surge-subscriptions.md), [ADR 0017](docs/kb/decisions/0017-built-in-surge-profile-artifact.md), [ADR 0018](docs/kb/decisions/0018-manual-app-generator-refresh.md), and [`architecture/data-flows.md`](docs/kb/architecture/data-flows.md).

### Sys preset (OS init)

1. Create `presets/sys/<os_id>/shine.toml` with `version = 2`:
   ```toml
   description = "One-line description of this OS init preset."
   default_profile = "recommended"
   version = 2

   [[items]]
   id = "neovim"
   label = "Neovim"
   description = "Install Neovim"

   [items.detect]
   kind = "command"
   command = "nvim"
   version_args = ["--version"]

   [items.install]
   kind = "package"
   provider = "homebrew" # homebrew-cask, apt, or winget
   package = "neovim"

   [profiles.recommended]
   items = ["neovim"]
   ```
2. Detection is `command`, `path`, or `any`. Package providers are fixed ensure-present actions and
   never upgrade. Complex installs use `[items.install] kind = "script"` with one item-local script,
   normal exit status.
3. Put OS-wide shell setup in `profile/base.pre.*` / `base.post.*`. Declare item integrations with
   exactly one of `path`, `env`, `eval`, `source`, `aliases`, or `fragment`; complex integration lives
   in `profile/<item>.*`. Named `[profiles.*]` tables select items only.
4. Every init item must declare both `detect` and `install`. v1 manifests and unknown versions fail
   before execution; do not add a platform dispatcher or status/update protocol.
5. `cargo build` re-embeds. Verify with `shine sys list`, `shine sys info <ITEM>`, and
   `shine sys bootstrap <ITEM> --dry-run`.
