# shine

A fast Rust CLI tool for managing shell environment presets.

`shine` embeds reusable shell scripts (proxy setup, etc.) into its binary and installs them to a predictable location (`~/.shine/`), creating symlinks in `~/.shine/bin/` so you can add one directory to your `PATH` and gain instant access to all managed scripts.

## Features

- **Embedded presets** — shell scripts are compiled into the binary; no internet required after installation
- **Symlink-based bin directory** — `~/.shine/bin/` holds flat symlinks to installed scripts; add it to `PATH` once
- **Auto PATH setup** — `install` appends `~/.shine/bin` to your shell config automatically
- **Category install** — install all presets or a specific subset (e.g. `proxy`)
- **Safe uninstall** — removes only shine-managed files; user-created files are never touched
- **Dry-run support** — preview any destructive operation before it runs
- **TOML config** — `~/.shine/config.toml` with comment preservation on updates
- **Release update check** — checks GitHub Releases at runtime with a 24h cache
- **Multi-shell support** — bash, zsh, fish, powershell, elvish

## Installation

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Or install from source:

```bash
cargo install --path cli
```

Or build from source:

```bash
cargo build --release
# Binary at: target/release/shine
```

## Usage

### List available presets

```bash
shine shell list
```

Shows all bundled preset categories and a description of each script:

```
Available shell preset categories:

  proxy (2 scripts)
    set_proxy     Set HTTP/HTTPS proxy environment variables.
                  ...
    uset_proxy    Unset all proxy environment variables.
                  ...

  tools (1 script)
    test_tools    Verify shine-installed shell tools are callable.
                  ...
```

### Install shell presets

```bash
shine shell install            # install all categories
shine shell install proxy      # install only the proxy category
```

Extracts embedded shell scripts to `~/.shine/presets/shell/`, creates symlinks in `~/.shine/bin/`, and appends a PATH entry to your shell config (`~/.zshrc`, `~/.bashrc`, `~/.config/fish/config.fish`, etc.):

```
Presets (shell): 3 created, 0 skipped
Bin links: 3 created, 0 skipped, 0 conflicts
Shell config (~/.zshrc): PATH updated
```

Running `install` again is safe — existing files, correct symlinks, and an already-configured PATH entry are all skipped.

### Runtime update policy

`shine` checks the latest GitHub Release for `biulight/shine` before executing commands and caches the result for 24 hours under `~/.shine/`.

- Newer `major` or `minor` release: prints an upgrade reminder and continues
- Newer `patch` release: requires you to upgrade before continuing
- Network/API/cache failures: silently skipped, command execution continues

Manual commands:

```bash
shine update   # force-check the latest release, do not install
shine upgrade  # download and install the latest release for this platform
```

### install.sh options

`install.sh` defaults to installing `shine` into `~/.local/bin/shine` without editing your shell config.

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=0.4.1 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

### Uninstall shell presets

```bash
shine shell uninstall
```

Removes shine-managed symlinks from `~/.shine/bin/`, preset files from `~/.shine/presets/shell/`, and the PATH entry from your shell config. User-created files are never removed.

```bash
shine shell uninstall --dry-run   # preview without changes
shine shell uninstall --purge     # also remove empty managed directories
```

`--purge` removes `~/.shine/bin/` and `~/.shine/presets/shell/` if empty after uninstall. It never removes `~/.shine/config.toml` or the root `~/.shine/` directory.

## Bundled Presets

### shell/proxy — `set_proxy` / `uset_proxy`

One-command proxy management for the entire development environment.

**Set proxy:**

```bash
source set_proxy           # auto-detect SOCKS5 or fall back to HTTP
source set_proxy sock5     # force SOCKS5
source set_proxy http      # force HTTP
```

Configures simultaneously:
- Shell environment variables (`http_proxy`, `https_proxy`, `all_proxy`, …)
- Git global config (`http.proxy`, `https.proxy`)
- npm / yarn / pnpm proxy settings

Default ports: HTTP `6152`, SOCKS5 `6153` (edit `~/.shine/presets/shell/proxy/set_proxy.sh` to change).

**Unset proxy:**

```bash
source uset_proxy
```

Clears all proxy environment variables and removes git/npm/yarn/pnpm proxy config.

### shell/tools — `test_tools`

Verifies that shine-installed shell tools are callable from the current environment.

## Configuration

`~/.shine/config.toml` is created automatically on first run.

Override directories at runtime:

```bash
SHINE_CONFIG_DIR=/custom/path shine shell install   # override shine dir + presets dir
SHINE_PRESETS=/custom/presets shine shell install   # override presets dir only
```

Or persist a custom presets directory in `~/.shine/config.toml`:

```toml
presets_dir = "/custom/presets"
```

Priority: `SHINE_CONFIG_DIR` > `SHINE_PRESETS` > `config.toml[presets_dir]` > default.

## Directory Layout

```
~/.shine/
├── config.toml
├── bin/
│   ├── set_proxy        # symlink → presets/shell/proxy/set_proxy.sh
│   ├── uset_proxy       # symlink → presets/shell/proxy/uset_proxy.sh
│   └── test_tools       # symlink → presets/shell/tools/test_tools.sh
└── presets/
    └── shell/
        ├── proxy/
        │   ├── set_proxy.sh
        │   └── uset_proxy.sh
        └── tools/
            └── test_tools.sh
```

## Development

```bash
cargo nextest run --all-features   # tests (used by pre-commit)
cargo test                         # fallback
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo fmt
cargo deny check bans licenses sources
typos
```

### Workspace layout

```
shine/
├── cli/        # binary crate — CLI parsing, commands, config
│   ├── build.rs               # triggers rust-embed recompile on presets/ changes
│   └── src/
│       ├── main.rs
│       ├── bin_links.rs       # symlink management
│       ├── presets.rs         # embedded-asset extraction, list_categories
│       ├── config/            # Config struct, load/save, env-var priority chain
│       ├── commands/          # clap subcommand definitions
│       └── shells/            # ShellType, install/uninstall/list, PATH injection
├── utils/      # library crate — TOML comment-preserving migration
└── presets/    # shell scripts embedded into the binary at compile time
    └── shell/
        ├── proxy/
        └── tools/
```

## License

MIT OR Apache-2.0
