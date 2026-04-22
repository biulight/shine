# shine

A fast Rust CLI tool for managing shell environment presets.

`shine` embeds reusable shell scripts (proxy setup, etc.) into its binary and installs them to a predictable location (`~/.shine/`), creating symlinks in `~/.shine/bin/` so you can add one directory to your `PATH` and gain instant access to all managed scripts.

## Features

- **Embedded presets** — shell scripts are compiled into the binary; no internet required after installation
- **Symlink-based bin directory** — `~/.shine/bin/` holds flat symlinks to installed scripts; add it to `PATH` once
- **Safe uninstall** — removes only shine-managed files; user-created files are never touched
- **Dry-run support** — preview any destructive operation before it runs
- **TOML config** — `~/.shine/config.toml` with comment preservation on updates
- **Multi-shell support** — bash, zsh, fish, powershell, elvish

## Installation

```bash
cargo install --path cli
```

Or build from source:

```bash
cargo build --release
# Binary at: target/release/cli (rename/alias to `shine`)
```

Then add `~/.shine/bin` to your shell's `PATH`:

```bash
# bash / zsh
echo 'export PATH="$HOME/.shine/bin:$PATH"' >> ~/.zshrc

# fish
fish_add_path ~/.shine/bin
```

## Usage

### Install shell presets

```bash
shine shell install
```

Extracts embedded shell scripts to `~/.shine/presets/shell/` and creates symlinks in `~/.shine/bin/`.

Output example:

```
Presets (shell): 2 created, 0 skipped
Bin links: 2 created, 0 skipped, 0 conflicts
```

Running `install` a second time is safe — existing files and correct symlinks are skipped.

### Uninstall shell presets

```bash
shine shell uninstall
```

Removes shine-managed symlinks from `~/.shine/bin/` and embedded-asset preset files from `~/.shine/presets/shell/`. User-created files are never removed.

```bash
# Preview what would be removed without touching anything
shine shell uninstall --dry-run

# Also remove empty managed directories after uninstall
shine shell uninstall --purge
```

`--purge` removes `~/.shine/bin/` and `~/.shine/presets/shell/` if they are empty after uninstall. It never removes `~/.shine/config.toml` or the root `~/.shine/` directory.

## Bundled Presets

### shell/proxy — `set_proxy.sh` / `uset_proxy.sh`

One-command proxy management for the entire development environment.

**Set proxy:**

```bash
source ~/.shine/bin/set_proxy.sh          # auto-detect SOCKS5 or fall back to HTTP
source ~/.shine/bin/set_proxy.sh sock5    # force SOCKS5
source ~/.shine/bin/set_proxy.sh http     # force HTTP
```

Configures simultaneously:
- Shell environment variables (`http_proxy`, `https_proxy`, `all_proxy`, …)
- Git global config (`http.proxy`, `https.proxy`)
- npm / yarn / pnpm proxy settings

Default ports: HTTP `6152`, SOCKS5 `6153` (edit `~/.shine/presets/shell/proxy/set_proxy.sh` to change).

**Unset proxy:**

```bash
source ~/.shine/bin/uset_proxy.sh
```

Clears all proxy environment variables and removes git/npm/yarn/pnpm proxy config.

## Configuration

`~/.shine/config.toml` is created automatically on first run. Currently it stores the schema version; future releases will expose user-configurable options here.

Override the config directory at runtime:

```bash
SHINE_CONFIG_DIR=/custom/path shine shell install
```

## Directory Layout

```
~/.shine/
├── config.toml          # user configuration
├── bin/
│   ├── set_proxy.sh     # symlink → presets/shell/proxy/set_proxy.sh
│   └── uset_proxy.sh    # symlink → presets/shell/proxy/uset_proxy.sh
└── presets/
    └── shell/
        └── proxy/
            ├── set_proxy.sh
            └── uset_proxy.sh
```

## Development

```bash
# Run tests
cargo test --all

# Lint
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt --all
```

### Workspace layout

```
shine/
├── cli/        # binary crate — CLI parsing, commands, config
│   └── src/
│       ├── main.rs
│       ├── bin_links.rs   # symlink management
│       ├── presets.rs     # embedded-asset extraction / removal
│       ├── config/        # Config struct, load/save, TOML migration
│       ├── commands/      # clap subcommand definitions
│       └── shells/        # shell-specific handlers
└── utils/      # library crate — TOML comment-preserving migration
```

## License

MIT OR Apache-2.0
