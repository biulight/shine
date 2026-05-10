# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

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
├── cli/          # Main binary crate
│   ├── build.rs  # cargo:rerun-if-changed=../presets (rust-embed trigger)
│   └── src/
│       ├── main.rs           # CLI entry, command routing
│       ├── commands/
│       │   ├── mod.rs        # Clap subcommand enums (ShellCommands, AppCommands, etc.)
│       │   ├── app.rs        # AppCommands enum
│       │   ├── env.rs        # EnvCommands enum
│       │   ├── preset.rs     # ExportCommand, LinkCommand structs
│       │   ├── self_install.rs # SelfCommands enum (install, upgrade)
│       │   ├── shell.rs      # ShellCommands enum
│       │   └── sys.rs        # SysCommands enum
│       ├── apps/
│       │   ├── mod.rs        # App install/uninstall/list orchestration
│       │   ├── metadata.rs   # shine.toml manifest parsing (AppCategory, AppFile)
│       │   ├── annotation.rs # shine-dest: comment annotation parser
│       │   ├── file_ops.rs   # File copy, backup (*.shine.bak), restore
│       │   ├── manifest.rs   # ~/.shine/app-manifest.toml tracking
│       │   └── transforms/   # File content transforms: jsonc-to-json, template
│       ├── env/
│       │   ├── mod.rs        # EnvConfig: [env] table in config.toml, @@VAR@@ substitution
│       │   └── upgrade.rs    # Re-apply env template transforms to installed presets
│       ├── shells/
│       │   ├── mod.rs        # ShellType, handle_install/uninstall/list, PATH injection
│       │   └── metadata.rs   # ShellCategory/ShellFile parsing from shine.toml or .sh files
│       ├── sys/
│       │   └── mod.rs        # sys list/init: OS detection, profile selection, init.sh execution
│       ├── config/           # Config struct, load/save, env-var priority chain
│       ├── presets.rs        # rust-embed asset extraction, list_categories, parse_script_description
│       ├── bin_links.rs      # Symlink management in ~/.shine/bin/
│       ├── clear.rs          # Clear stale runtime state after schema changes
│       ├── colors.rs         # Terminal color helpers
│       ├── list.rs           # Top-level `shine list` and status views
│       ├── secret.rs         # GPG encrypt/decrypt for env secrets
│       ├── show.rs           # `shine info <TARGET>` content display
│       ├── update_check.rs   # GitHub release version check, 24h cache
│       └── version.rs        # Version string formatting
├── utils/        # Library crate: TOML comment-preserving sync (utils::sync_table)
└── presets/      # Embedded assets (compiled into binary via rust-embed)
    ├── shell/
    │   ├── agent/   cc.sh, shine.toml  (needs_source=true; installed as `ccenv`)
    │   ├── proxy/   set_proxy.sh, uset_proxy.sh, shine.toml
    │   └── tools/   test_tools.sh
    ├── app/
    │   ├── archey4/    config.json, shine.toml
    │   ├── docker/     daemon.jsonc, shine.toml
    │   ├── fastfetch/  config.jsonc, shine.toml
    │   ├── ghostty/    config.ghostty, shine.toml
    │   ├── git/        gitconfig  (shine-dest: ~/.gitconfig)
    │   ├── JetBrains/  shine.toml
    │   ├── starship/   starship.toml
    │   └── vim/        shine.toml, vimrc, _machine_specific.vim
    └── sys/
        ├── macos/   init.sh, shine.toml
        └── ubuntu/  init.sh, shine.toml
```

### Command routing

`main.rs` → `Commands` enum → module handlers:

| Top-level command | Handler module |
|---|---|
| `shell list/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `sys list/init` | `cli/src/sys/` |
| `env show/set/get/decrypt/encrypt` | `cli/src/env/` |
| `list` | `cli/src/list.rs` |
| `info <TARGET>` | `cli/src/show.rs` |
| `export` / `link` / `unlink` | `main.rs` inline handlers |
| `init` | `main.rs` inline handler |
| `self install/upgrade` | `main.rs` + `update_check.rs` |
| `update` / `upgrade` | `main.rs` + `update_check.rs` |
| `clear` | `cli/src/clear.rs` |
| `completions` | `main.rs` inline (clap_complete) |

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

`shine env encrypt`/`shine env decrypt` use GPG (`secret.rs`) to store secrets as base64-encoded ciphertext in the `[env]` table.

### File transforms (`apps/transforms/`)

Two transforms can be applied to preset files at install time (declared in `shine.toml` as `transforms = [...]`):

- **`jsonc-to-json`** — strips JSONC-style comments so pure-JSON apps can consume the output (used by `docker/daemon.jsonc`, `fastfetch/config.jsonc`).
- **`template`** — substitutes `@@VAR_NAME@@` placeholders from the active `[env]` config table.

Transforms compose in declaration order: `transforms = ["jsonc-to-json", "template"]`.

### Sys preset flow (`shine sys init`)

1. `sys::detect_os_id()` — reads `std::env::consts::OS`; on Linux reads `ID=` from `/etc/os-release`.
2. `presets::extract_prefix("sys/<os_id>", presets_dir)` — unpacks `init.sh` + `shine.toml` for the detected OS.
3. `sys::load_sys_preset` — parses `shine.toml` for `description`, `[[items]]`, `[profiles.*]`, and `default_profile`.
4. In interactive mode: `dialoguer::MultiSelect` lets the user pick init items. Non-interactive mode requires `default_profile`.
5. Calls `bash <presets_dir>/sys/<os>/init.sh <item_id...>` with selected item IDs as arguments.

`shine sys init --preset <PROFILE>` bypasses interactive selection. `--dry-run` prints the command and script content without executing.

### Config (`~/.shine/config.toml`)

`Config::load_or_init()` resolves directories with this priority:
1. `SHINE_CONFIG_DIR` env var — overrides both shine dir and presets dir
2. `SHINE_PRESETS` env var — overrides presets dir only
3. `presets_dir` key in `config.toml`
4. Default: `~/.shine/` (shine dir), `~/.shine/presets/` (presets dir)

Config is saved via `utils::sync_table` which preserves existing TOML comments while updating values.

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
`bin_dir` paths under `home_dir` are expressed as `$HOME/...` for portability. Fish uses `fish_add_path` instead. `remove_path_from_shell_config` deletes the block precisely, including the preceding blank line separator.

### `shine shell list`

Reads embedded assets, groups them by immediate subdirectory under `shell/`, and displays per-script descriptions. If a category has `shine.toml`, the metadata file drives file listing and command names. Without `shine.toml`, descriptions are parsed from the leading comment block of each `.sh` file (lines starting with `# ` after the shebang, until the first non-comment line).

Shell categories can declare `needs_source = true` in `shine.toml` to mark a script as requiring `source` (not direct execution). These are exposed as shell functions rather than symlinked commands (e.g., `ccenv` from `presets/shell/agent/cc.sh`).

## Git Push Policy

**Never `git push` to the remote without explicit user approval.** Commit locally, then stop and let the user review before pushing. This applies to branch pushes, tag pushes, and force-pushes.

## CHANGELOG

Do **not** use `git cliff` to generate CHANGELOG entries. Write entries manually based on the actual changes in the release. Follow the existing format:

```
## [x.y.z] — YYYY-MM-DD

### Features / Bug Fixes / Internal / Docs

- Plain-English description of what changed and why
```

Keep entries concise and user-facing. Internal refactors can be grouped under **Internal**.

## Release Versioning

When deciding whether to bump the release version, always compare the current branch against the most recent **stable** release tag, not the moving `preview` tag.

- Treat `preview` as a prerelease channel marker only; it is not the previous release baseline.
- Use the latest `v*` tag such as `v0.20.0` as the baseline for release notes and version-bump decisions.
- Do not rely on `git describe --tags --abbrev=0` by itself in this repo, because it may resolve to `preview`.
- Prefer commands that filter for stable tags explicitly, for example `git tag --list 'v*' --sort=-version:refname | head -1`.
- If there are user-facing features since the last stable tag, bump `minor`; if there are only user-facing fixes, bump `patch`.

### Commit scope convention for internal fixes

Fix commits that exist only because new feature code in the same release introduced them (clippy noise, lint, formatting, typos) must use one of these scopes so `git cliff` automatically skips them:

| Scope | Example |
|-------|---------|
| `fix(lint): ...` | clippy allow/deny rule adjustment |
| `fix(clippy): ...` | clippy suggestion |
| `fix(fmt): ...` | rustfmt formatting |
| `fix(typo): ...` | spell-check fix in new code |
| `fix(build): ...` | build/compile error in new code |
| `fix(ci): ...` | CI pipeline fix |
| `fix(internal): ...` | any other non-user-facing cleanup |

Real user-facing bug fixes must **not** use these scopes — use the affected feature area instead (e.g. `fix(install): ...`, `fix(shell): ...`).

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

### Sys preset (OS init)

1. Create `presets/sys/<os_id>/init.sh` — a bash script that accepts item IDs as positional arguments.
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
3. `cargo build` re-embeds. Verify with `shine sys list` and `shine sys init --dry-run`.
