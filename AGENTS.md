# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

`shine` is a self-contained Rust CLI that bundles shell scripts, app config presets, and OS
bootstrap presets into one binary (rust-embed), installs them under `~/.shine/`, and supports
safe, manifest-tracked uninstall. Cargo workspace: `cli/` (binary + lib) and `utils/`.

## Where knowledge lives

| Need | Read |
|---|---|
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

## Commands

```bash
# Build
cargo build
cargo build --release          # binary at target/release/shine

# Run (dev)
cargo run -- shell list
cargo run -- shell install
cargo run -- shell install proxy
cargo run -- shell uninstall --dry-run
cargo run -- sys list
cargo run -- sys init --dry-run
cargo run -- env show
cargo run -- self upgrade --channel preview

# Test (pre-commit uses nextest)
cargo nextest run --all-features
cargo test                     # fallback without nextest

# Single test
cargo test shells::tests::install_then_uninstall_roundtrip
cargo nextest run -E 'test(install_then_uninstall)'

# Lint / format
cargo fmt
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo deny check bans licenses sources
typos                          # spell-check
```

Pre-commit runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check`, `typos`, and `cargo nextest run` on every commit. All must pass before committing.

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
├── cli/          # Main binary crate ("shine"), backed by a lib crate ("cli")
│   ├── build.rs  # cargo:rerun-if-changed=../presets (rust-embed trigger)
│   └── src/
│       ├── lib.rs            # Module tree root for the `cli` library crate
│       ├── main.rs           # Bin crate root: `fn main`, `run()` dispatch, `init` handler
│       ├── shim.rs           # Top-level install/reinstall/uninstall <category>:
│       │                     # infers shell vs app preset, prompts on conflict
│       ├── home.rs           # effective_home_dir (sudo-aware), tilde/full path expansion
│       ├── presets_commands.rs # export/link/unlink, overlay link/unlink/show
│       ├── self_install.rs   # update/self-upgrade/upgrade-installed-configs,
│       │                     # atomic self-install binary copy
│       ├── commands/
│       │   ├── mod.rs        # Clap subcommand enums (ShellCommands, AppCommands, etc.)
│       │   ├── cli.rs        # Cli, Commands, CompletionShell/Commands, and the
│       │   │                 # other top-level clap arg types (lives in the lib
│       │   │                 # crate since completion.rs needs them)
│       │   ├── app.rs        # AppCommands enum
│       │   ├── env.rs        # EnvCommands enum
│       │   ├── preset.rs     # ExportCommand, LinkCommand structs
│       │   ├── self_install.rs # SelfCommands enum (install, upgrade)
│       │   ├── shell.rs      # ShellCommands enum
│       │   └── sys.rs        # SysCommands enum
│       ├── apps/
│       │   ├── mod.rs        # App install/uninstall/list orchestration
│       │   ├── report.rs     # Install/uninstall outcome print_* helpers
│       │   ├── upgrade.rs    # handle_upgrade_installed, stale-entry cleanup
│       │   ├── metadata.rs   # shine.toml manifest parsing (AppCategory, AppFile)
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
│       │   ├── commands.rs   # `shine env show/set/delete/get/decrypt/export/encrypt` handlers
│       │   ├── catalog.rs    # Known env-var metadata (description, sensitive) for `env show`
│       │   ├── identity.rs   # `shine env identity init/show`: age identity generation
│       │   │                 # (age-keygen / age-plugin-se --touch-id) and recipient inspection
│       │   ├── upgrade.rs    # Re-apply env template transforms to installed presets
│       │   └── workspace.rs  # `shine env seal/run`: workspace env files, `--with` injection
│       ├── git_pull.rs       # Safe FF-only pulls for Git-managed preset sources
│       ├── shells/
│       │   ├── mod.rs        # ShellType, handle_install/uninstall/list, link-conflict reporting
│       │   ├── profile.rs    # Managed profile file/PATH/sentinel-block install+removal
│       │   ├── template.rs   # @@VAR@@ template rendering for installed scripts
│       │   └── metadata.rs   # ShellCategory/ShellFile parsing from shine.toml or .sh files
│       ├── sys/
│       │   ├── mod.rs        # sys handle_* entry points, OS detection, init/apply orchestration
│       │   ├── model.rs      # SysManifest/SysItem/SysItemStatus/SysItemOutcome/SelectionSource, etc.
│       │   ├── run_manifest.rs # SysRunManifest/SysRunEntry: ~/.shine/sys-manifest.toml load/save
│       │   ├── manifest.rs   # Preset loading, parsing, and validation
│       │   ├── profile.rs    # Shell-profile install/merge/sentinel logic
│       │   ├── selection.rs  # Item-selection resolution (profile vs interactive)
│       │   ├── execution.rs  # Running sys items, parsing script output, run reports
│       │   └── resources.rs  # Built-in managed-resource drivers (split-dns, etc.)
│       ├── config/
│       │   ├── mod.rs        # Config struct + accessors, Default, new_for_test
│       │   ├── load.rs       # load_or_init, global/project layering, schema version read
│       │   ├── save.rs       # Atomic save, comment-preserving merge, sparse project diff
│       │   ├── env_layer.rs  # [env] table parsing, legacy env.toml migration, override files
│       │   └── discovery.rs  # Project-config discovery, SHINE_CONFIG_DIR/
│       │                     # SHINE_PRESETS priority chain
│       ├── presets.rs        # rust-embed asset extraction, list_categories, parse_script_description
│       ├── bin_links.rs      # Symlink management in ~/.shine/bin/
│       ├── status.rs         # Shared install-status row builders used by `list`/`info`
│       ├── clear.rs          # Clear stale runtime state after schema changes
│       ├── colors.rs         # Terminal color helpers
│       ├── serve.rs          # Local HTTP server for shine-managed resources under ~/.shine/http/
│       ├── list.rs           # Top-level `shine list` and status views
│       ├── path_display.rs   # Home-relative path formatting for terminal output
│       ├── secret/
│       │   ├── mod.rs        # BackendKind/EncryptRecipients, tagged-ciphertext router
│       │   │                 # (encrypt_secret/decrypt_secret); untagged = gpg, `age:` = age
│       │   ├── exec.rs       # Shared subprocess helpers (ensure_command, TempFile, base64
│       │   │                 # encode/decode) used by both backends below
│       │   ├── gpg.rs        # GPG-backed encrypt/decrypt, untagged base64 ciphertext
│       │   └── age.rs        # age-backed encrypt/decrypt, multi-recipient + Secure Enclave
│       │                     # (age-plugin-se) identity support, `age:`-tagged ciphertext
│       ├── show.rs           # `shine info <TARGET>` content display
│       ├── ssh/
│       │   ├── mod.rs        # `shine ssh`: wraps system ssh, arg splitting, session
│       │   │                 # bootstrap, wrapped remote command (env vars + EXIT trap)
│       │   ├── protocol.rs   # Wire format shared by agent.rs and remote_client.rs
│       │   ├── agent.rs      # Local transfer server (PutFile/GetFile), Unix socket or
│       │   │                 # loopback TCP (Windows) via LocalListener
│       │   ├── dir_transfer.rs # Directory tar build/extract, symlink-escape validation
│       │   └── remote_client.rs # `shine local download/upload` handlers (run on remote host)
│       ├── task/
│       │   ├── mod.rs        # `shine task` save/run/list/info/delete handlers,
│       │   │                 # direct (no-shell) argv exec + exit-code passthrough,
│       │   │                 # shell-quoted command rendering, task-name validation
│       │   └── manifest.rs   # TaskManifest: <shine_dir>/tasks.toml load/save/upsert
│       ├── test_support.rs   # Shared test-only env-var mutex (not cfg(test)-gated,
│       │                     # since #[cfg(test)] doesn't cross the lib/bin boundary)
│       ├── update_check.rs   # GitHub release version check, 24h cache
│       └── version.rs        # Version string formatting
├── utils/        # Library crate: shared helpers with no cli-crate dependencies
│   └── src/
│       ├── migration.rs      # TOML comment-preserving sync (utils::sync_table)
│       └── init_template.rs  # write_shine_toml_template (shared by `app init`/`shell init`)
└── presets/      # Embedded assets (compiled into binary via rust-embed)
    ├── shell/
    │   ├── agent/   cc.sh, cc.ps1, shine.toml  (needs_source=true; installed as `ccenv`; platform-scoped per shell family)
    │   ├── proxy/   set_proxy.sh, uset_proxy.sh, shine.toml
    │   └── utils/   copyfile.sh, shine.toml
    ├── app/
    │   ├── archey4/    config.json, shine.toml
    │   ├── docker-desktop/ settings-store.jsonc, shine.toml
    │   ├── docker-engine/  daemon.jsonc, shine.toml
    │   ├── fastfetch/  config.jsonc, shine.toml
    │   ├── ghostty/    config.ghostty, shine.toml
    │   ├── git/        gitconfig  (shine-dest: ~/.gitconfig; no shine.toml, uses annotation instead)
    │   ├── JetBrains/  shine.toml
    │   ├── surge/      custom-rules.sgmodule, shine.toml
    │   ├── starship/   starship.toml  (shine-dest: ~/.config/starship.toml; no shine.toml, uses annotation instead)
    │   └── vim/        shine.toml, vimrc, _machine_specific.vim
    └── sys/
        ├── macos/   init.sh, shine.toml
        ├── ubuntu/  init.sh, shine.toml
        └── windows/ init.ps1, shine.toml
```

### Command routing

`main.rs` → `Commands` enum → module handlers:

| Top-level command | Handler module |
|---|---|
| `shell list/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `sys list/init` | `cli/src/sys/` |
| `env show/set/get/decrypt/encrypt/identity` | `cli/src/env/` |
| `list` | `cli/src/list.rs` |
| `info <TARGET>` | `cli/src/show.rs` |
| `export` / `link` / `unlink` / `overlay` | `cli/src/presets_commands.rs` |
| `pull` / `update --pull` / `upgrade --pull` | `cli/src/git_pull.rs` + `main.rs` routing |
| `init` | `main.rs` inline handler |
| `self install/upgrade` | `cli/src/self_install.rs` + `update_check.rs` |
| `update` / `upgrade` | `cli/src/self_install.rs` + `update_check.rs` |
| `clear` | `cli/src/clear.rs` |
| `serve install/start/status/uninstall/url` | `cli/src/serve.rs` |
| `completions` | `main.rs` inline (clap_complete) |
| `ssh [SSH_ARGS]... <HOST> [COMMAND]` | `cli/src/ssh/mod.rs` |
| `local download/upload/status` | `cli/src/ssh/remote_client.rs` |
| `task save/run/list/info/delete` | `cli/src/task/` |
| `run <NAME>` (alias for `task run`) | `cli/src/task/` |

### Key data flow

**Install** (`shine shell install [CATEGORY]`):
1. `presets::extract_prefix("shell[/category]", presets_dir)` — unpacks embedded assets to `~/.shine/presets/shell/`
2. `bin_links::link_executables(bin_dir, sources)` — creates flat symlinks in `~/.shine/bin/`
3. `shells::append_path_to_shell_config` — appends a sentinel-guarded `export PATH` block to `~/.zshrc` (or equivalent)

**Uninstall**:
1. `bin_links::unlink_managed` — removes only symlinks pointing into the managed presets dir
2. `presets::remove_prefix` — removes only embedded-asset files (user files are never touched)
3. `shells::remove_path_from_shell_config` — removes the sentinel block; skipped on `--dry-run`

### Env variable substitution

`shine env set KEY VALUE` writes to the `[env]` table in `config.toml`. App (and shell) preset files that declare `transforms = ["template"]` in their `shine.toml` have `@@KEY@@` placeholders replaced at install/upgrade time. Run `shine upgrade` after changing env vars to re-apply to installed presets.

`shine env encrypt`/`shine env decrypt` store secrets as base64 ciphertext in the `[env]` table,
routed through `secret::encrypt_secret`/`decrypt_secret` (`secret/mod.rs`) to either the GPG
(`secret/gpg.rs`, untagged ciphertext) or age (`secret/age.rs`, `age:`-tagged ciphertext, with
optional Apple Touch ID via `age-plugin-se`) backend. Decryption always routes purely on the
ciphertext's tag, never on config — see
[ADR 0008](docs/kb/decisions/0008-age-secret-backend-tagged-ciphertext.md). `shine env identity
init/show` (`env/identity.rs`) manages the age identity file consulted via
`Config::age_identities()`.

### File transforms (`install_core/transforms/`)

Two transforms can be applied to preset files at install time (declared in `shine.toml` as `transforms = [...]`):

- **`jsonc-to-json`** — strips JSONC-style comments so pure-JSON apps can consume the output (used by `docker-engine/daemon.jsonc`, `docker-desktop/settings-store.jsonc`, `fastfetch/config.jsonc`).
- **`template`** — substitutes `@@VAR_NAME@@` placeholders from the active `[env]` config table.

Transforms compose in declaration order: `transforms = ["jsonc-to-json", "template"]`.

### SSH session transfer flow (`shine ssh` / `shine local`)

See [`docs/ssh-local-transfer-prd.md`](docs/ssh-local-transfer-prd.md) for the full design.
Phases 1-3 implemented: file/directory transfers, progress output, and
`shine local status`. Windows support (local side only — see step 8) added
on top of the macOS/Linux implementation.

1. `shine ssh [SSH_ARGS]... <HOST> [COMMAND]` generates a session id + token
   and spawns `ssh` with `-t -R <remote-sock>:<local-forward-target>`
   prepended to the user's own args (`ssh::split_ssh_args` locates the
   destination/command boundary without reinterpreting ssh's own option
   semantics). `<remote-sock>` is always a Unix socket path under `/tmp` on
   the remote host (assumed Linux/macOS); `<local-forward-target>` is
   platform-dependent — see step 8.
2. The remote command is replaced with a wrapper that sets
   `SHINE_SSH_SESSION`/`SHINE_SSH_TOKEN`/`SHINE_SSH_REMOTE_SOCK` via `env`
   (not `SetEnv`/`SendEnv`, which most `sshd_config`s reject), then `exec`s
   the user's original remote command or their login shell.
3. sshd does **not** clean up the forwarded remote socket file on
   disconnect (verified against a real host via `scripts/spike-ssh-forward.sh`
   before implementation) — the wrapper registers its own `trap ... EXIT`.
4. `shine local download/upload` (run on the remote host) reads those env
   vars, dials the forwarded socket, and speaks the framed protocol in
   `ssh::protocol` (`PutFile`/`GetFile`/`Preview` for `--dry-run`) against
   the local agent in `ssh::agent`, which resolves destinations against the
   session's local working directory per the PRD's default-target rules.
   The only authorization on a request is the session token, which travels
   to the remote host as plain argv/environ (`env SHINE_SSH_TOKEN=...`) and
   is therefore readable by other local users there via `ps eww` — so the
   agent does not otherwise trust wire-supplied fields: `PutFile.filename`
   (meant to always be a bare basename) is rejected unless it is exactly
   one normal path component (`agent::ensure_single_path_component`),
   preventing a forged request from writing outside the session directory
   via `..` or an absolute path, and `dest_hint`/`source_hint` are expanded
   with `~`-only substitution (`home::tilde_expand`), not the full
   `${VAR}`-style expansion used for locally-typed paths elsewhere in the
   crate, since that would let a forged hint pull values out of the local
   agent process's own environment.
5. Directories are staged as an uncompressed tar archive in a local temp
   file (`ssh::dir_transfer::build_tar_to_temp_file`/`extract_tar_from_file`,
   run on `spawn_blocking` since the `tar` crate is synchronous) and sent
   through the same `PutFile`/`GetFile` byte-streaming path with an
   `is_dir` flag — never buffered fully in memory, and never assembled
   in-memory as a whole archive. `tar::Entry::unpack_in` already rejects
   absolute paths and `..` traversal in an entry's own path, but does
   **not** validate a *symlink's target*, so `dir_transfer` adds its own
   check enforcing the PRD's chosen policy: relative symlinks that resolve
   inside the transferred tree are kept, absolute or escaping ones reject
   the whole transfer. Non-file/dir/symlink entry types (hard links,
   device nodes, FIFOs) are rejected outright. An existing destination
   directory is rejected without `--force`; with `--force` the archive is
   merged into it (existing files not present in the archive are kept,
   matching the PRD's stated `--force` semantics for directories).
6. `protocol::copy_exact_with_progress` reports cumulative bytes copied
   after each chunk; `remote_client::ProgressPrinter` (throttled to ~150ms)
   renders a single overwritten stderr line only when
   `console::user_attended_stderr()` is true, per the PRD's requirement
   that non-TTY environments get a stable single-line result with no live
   redraw. `agent`'s side of transfers has no progress output — the
   command always runs (and its stdout/stderr are visible) on the remote
   host, not locally.
7. `shine local status` sends a `Status` request over the same forwarded
   socket; the agent replies with the session's local working directory,
   and the client also reports the session id (from `SHINE_SSH_SESSION`)
   and negotiated protocol version. If the agent is unreachable, it
   reports that instead of erroring, so the command doubles as a
   liveness check without needing a live session.
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
   per-connection protocol logic (`handle_connection`, `handle_put_file`,
   `handle_get_file`) stays transport-agnostic and unchanged for both
   platforms.
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

### Sys preset flow (`shine sys init`)

1. `sys::detect_os_id()` — reads `std::env::consts::OS`; on Linux reads `ID=` from `/etc/os-release`.
2. `presets::extract_prefix("sys/<os_id>", presets_dir)` — unpacks `init.sh` + `shine.toml` for the detected OS.
3. `sys::load_sys_preset` — parses `shine.toml` for `description`, `[[items]]`, `[profiles.*]`, and `default_profile`.
4. In interactive mode: `dialoguer::MultiSelect` lets the user pick init items. Non-interactive mode requires `default_profile`.
5. Calls `bash <presets_dir>/sys/<os>/init.sh <item_id>` once per selected item, then calls `bash <presets_dir>/sys/<os>/init.sh __shine_finalize` so shared shell/profile integration runs once.

`shine sys init --preset <PROFILE>` bypasses interactive selection. `--dry-run` prints the command and script content without executing.

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

### Local HTTP resources

`shine serve start` serves files from `~/.shine/http/` on a single loopback HTTP server
(`127.0.0.1:6174` by default). On macOS, `shine serve install` registers that same server as one
user launchd service; it is intentionally global, with no per-app argument. App presets that need
stable local URLs should install files under that tree (for example
`~/.shine/http/app/surge/custom-rules.sgmodule`) and use `shine serve url <path>` to print the
URL. Do not start one HTTP service per app preset.

### rust-embed and presets

`PresetAssets` (in `presets.rs`) embeds everything under `presets/` at compile time. `build.rs` registers `cargo:rerun-if-changed=../presets` so cargo recompiles when preset files change — without this, new/modified scripts won't appear in the binary.

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

Shell categories can declare `needs_source = true` in `shine.toml` to mark a script as requiring `source` (not direct execution). These are exposed as shell functions rather than symlinked commands. Entries can also declare `platforms = ["unix"]` or `platforms = ["windows"]` to ship different source files for the same command name on different platforms (e.g., `ccenv` from `presets/shell/agent/cc.sh` on Unix and `presets/shell/agent/cc.ps1` on Windows).

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

### App preset category

Prefer `shine.toml` metadata over legacy `shine-dest:` annotations for new categories. Place `shine.toml` in `presets/app/<category>/` with at minimum `dest = "~/<path>"`. Add `transforms = ["jsonc-to-json"]` for JSONC files or `transforms = ["template"]` for files with `@@VAR_NAME@@` env placeholders.

App categories may declare a `post_upgrade = { command = "...", args = ["..."] }` hook to run a direct argv command after `shine upgrade` actually updates or installs at least one file in that category. Hooks are not run during `app install`, and external presets require `allow_app_hooks = true` in config before hooks execute.

### Sys preset (OS init)

1. Create `presets/sys/<os_id>/init.sh` — a bash script that accepts one item ID as `$1`; support `__shine_finalize` if the preset needs shared profile or shell integration.
2. Create `presets/sys/<os_id>/shine.toml`:
   ```toml
   description = "One-line description of this OS init preset."
   default_profile = "recommended"

   [[items]]
   id = "neovim"
   label = "Neovim"
   description = "Install Neovim"

   [profiles.recommended]
   items = ["neovim"]
   ```
3. Emit compact status events from scripts with `printf 'SHINE_SYS_STATUS\t%s\t%s\n' "already-installed" "detail"`. Supported states are `installed`, `already-installed`, `skipped`, `updated`, `needs-action`, `completed`, and `failed`; other output is rendered as indented logs.
4. `cargo build` re-embeds. Verify with `shine sys list` and `shine sys init --dry-run`.
