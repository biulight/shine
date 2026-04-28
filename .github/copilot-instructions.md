# Copilot Instructions

## Build, Test & Lint

```bash
# Build
cargo build
cargo build --release          # binary at target/release/shine

# Run (dev)
cargo run -- shell list
cargo run -- shell install proxy
cargo run -- shell uninstall --dry-run

# Test
cargo nextest run --all-features   # used by pre-commit
cargo test                         # fallback without nextest

# Single test
cargo nextest run -E 'test(install_then_uninstall)'
cargo test shells::tests::install_then_uninstall_roundtrip

# Lint / format
cargo fmt
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo deny check bans licenses sources
typos
```

Pre-commit runs all of the above on every commit. All must pass before committing.

## Architecture

Cargo workspace with two crates:

- **`cli/`** — the `shine` binary (Clap-based, async with Tokio)
- **`utils/`** — library with `utils::migration::sync_table`, a TOML comment-preserving table updater

Preset files under `presets/` are compiled into the binary via `rust_embed` (`PresetAssets` in `cli/src/presets.rs`). `cli/build.rs` registers `cargo:rerun-if-changed=../presets` so cargo re-embeds when preset files change.

### Command routing

`main.rs` → `Commands` enum → module handlers:

| Top-level command | Handler module |
|---|---|
| `shell list/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `update` / `upgrade` | `cli/src/update_check.rs` |
| `check` | `cli/src/check.rs` |

### Shell preset install flow

1. `presets::extract_prefix("shell[/category]", presets_dir)` — unpacks embedded `.sh` files to `~/.shine/presets/shell/`, sets executable bit
2. `bin_links::link_executables(bin_dir, sources)` — creates flat symlinks in `~/.shine/bin/`
3. `shells::append_path_to_shell_config` — appends a sentinel-guarded `export PATH` block to `~/.zshrc` (or equivalent); Fish uses `fish_add_path`

Uninstall is the exact reverse; `remove_prefix` only touches files known to `PresetAssets` — user files are never deleted.

### App preset destination resolution

Two modes, tried in order per category:

1. **`shine.toml` metadata** (`presets/app/<CATEGORY>/shine.toml`): declares `description`, `dest` (required, must be absolute after `~` expansion), and optional `[[files]]` entries with `source`/`target`/`description`.
2. **Legacy file-level annotation**: first non-shebang comment line `# shine-dest: ~/.gitconfig` (shell/TOML/INI) or `" shine-dest: ~/.ideavimrc` (VimScript). Without any annotation, destination falls back to `<app_default_dest_root>/<CATEGORY>/<FILE>`.

Existing unmanaged files are moved to `*.shine.bak` before install; uninstall restores backups automatically. Installed files are tracked in `~/.shine/app-manifest.toml`.

### Config priority chain

`Config::load_or_init()` resolves directories in this order:

1. `SHINE_CONFIG_DIR` env var — overrides both shine dir and presets dir
2. `SHINE_PRESETS` env var — overrides presets dir only
3. `presets_dir` key in `~/.shine/config.toml`
4. Default: `~/.shine/` (shine dir), `~/.shine/presets/` (presets dir)

Config is written via `utils::migration::sync_table` to preserve TOML comments on update.

## Key Conventions

### Adding a shell preset category

1. Create `presets/shell/<category>/your_script.sh` with `#!/bin/bash` shebang.
2. Add a description comment block immediately after the shebang (lines starting with `# ` until first non-comment line) — this is what `shine shell list` displays.
3. `cargo build` re-embeds automatically.

### Adding an app preset category

Prefer `shine.toml` metadata over legacy `shine-dest:` annotations for new categories. Place `shine.toml` in `presets/app/<category>/` with at minimum `dest = "~/<path>"`.

### Commit message scopes

Internal-only fix commits (clippy, fmt, typos, build errors introduced by new feature code in the same release) must use scoped prefixes so `git cliff` skips them in the changelog: `fix(clippy):`, `fix(fmt):`, `fix(typo):`, `fix(build):`, `fix(lint):`, `fix(ci):`, `fix(internal):`.

Real user-facing bug fixes must **not** use these scopes — use the feature area instead (`fix(install):`, `fix(shell):`, etc.).

### Changelog

Do **not** use `git cliff` to generate entries. Write CHANGELOG.md entries manually using the existing format:

```
## [x.y.z] — YYYY-MM-DD

### Features / Bug Fixes / Internal / Docs

- Plain-English description of what changed and why
```

### Git push policy

**Never `git push` without explicit user approval.** Commit locally and stop.
