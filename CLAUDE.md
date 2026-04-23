# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

# CHANGELOG (git-cliff)
git cliff --output CHANGELOG.md
```

Pre-commit runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check`, `typos`, and `cargo nextest run` on every commit. All must pass before committing.

## Architecture

### Workspace layout

```
shine/
├── cli/          # Main binary crate
│   ├── build.rs  # cargo:rerun-if-changed=../presets (rust-embed trigger)
│   └── src/
│       ├── main.rs        # CLI entry, command routing
│       ├── commands/      # Clap subcommand enums (ShellCommands, AppCommands)
│       ├── config/        # Config struct, load/save, env-var priority chain
│       ├── presets.rs     # rust-embed asset extraction, list_categories, parse_script_description
│       ├── bin_links.rs   # Symlink management in ~/.shine/bin/
│       └── shells/        # ShellType, handle_install/uninstall/list, PATH injection
├── utils/        # Library crate: TOML comment-preserving sync (utils::sync_table)
└── presets/      # Embedded shell scripts (compiled into binary via rust-embed)
    └── shell/
        ├── proxy/   set_proxy.sh, uset_proxy.sh
        └── tools/   test_tools.sh
```

### Key data flow

**Install** (`shine shell install [CATEGORY]`):
1. `presets::extract_prefix("shell[/category]", presets_dir)` — unpacks embedded assets to `~/.shine/presets/shell/`
2. `bin_links::link_executables(bin_dir, sources)` — creates flat symlinks in `~/.shine/bin/`
3. `shells::append_path_to_shell_config` — appends a sentinel-guarded `export PATH` block to `~/.zshrc` (or equivalent)

**Uninstall**:
1. `bin_links::unlink_managed` — removes only symlinks pointing into the managed presets dir
2. `presets::remove_prefix` — removes only embedded-asset files (user files are never touched)
3. `shells::remove_path_from_shell_config` — removes the sentinel block; skipped on `--dry-run`

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

Reads embedded assets, groups them by immediate subdirectory under `shell/`, and displays per-script descriptions parsed from the leading comment block of each `.sh` file (lines starting with `# ` after the shebang, until the first non-comment line).

## Adding a new preset category

1. Create `presets/shell/<category>/your_script.sh` with a `#!/bin/bash` shebang and a multi-line `# description` comment block immediately after it.
2. `cargo build` will re-embed automatically (tracked by `build.rs`).
3. `shine shell list` will display the new category; `shine shell install <category>` will install it.
