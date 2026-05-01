# shine

A fast Rust CLI tool for managing shell environment presets.

`shine` embeds reusable shell scripts (proxy setup, etc.) into its binary and installs them to a predictable location (`~/.shine/`), creating symlinks in `~/.shine/bin/` so you can add one directory to your `PATH` and gain instant access to all managed scripts.

## Features

- **Embedded presets** — shell scripts and app configs are compiled into the binary; no internet required after installation
- **External presets** — point `presets_dir` at your own directory (e.g. a dotfiles repo) and `shine` reads from there instead; `shine presets export` seeds it with the built-ins
- **Symlink-based bin directory** — `~/.shine/bin/` holds flat symlinks to installed scripts; add it to `PATH` once
- **Auto PATH setup** — `install` appends `~/.shine/bin` to your shell config automatically
- **Category install/uninstall** — install or uninstall all presets or a specific subset (e.g. `proxy`)
- **Installed-only view** — `shine list` shows only what is currently set up on this machine
- **Safe uninstall** — removes only shine-managed files; user-created files are never touched
- **Dry-run support** — preview any destructive operation before it runs
- **TOML config** — `~/.shine/config.toml` with comment preservation on updates
- **App preset installer** — install annotated config files like `~/.gitconfig` or `~/.config/starship/starship.toml`
- **Release update check** — checks GitHub Releases at runtime with a 24h cache
- **Multi-shell support** — bash, zsh, fish, powershell, elvish

## Planning Workflow

Repository planning is managed in GitHub with a lightweight issue-based flow:

- Open ideas with the `Idea / Plan` issue template
- Promote accepted work into `Task` issues
- Track state with `status:` labels
- Use milestones only for release-relevant work

The full workflow lives in [`docs/PLAN.md`](docs/PLAN.md).

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

### List available shell presets

```bash
shine shell list
```

```
Shell Preset Categories

  proxy  2 scripts
    setproxy      Set HTTP/HTTPS proxy environment variables.
                  ...
    usetproxy     Unset all proxy environment variables.
                  ...

  tools  1 script
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
Shell Presets  3 created
Bin Links      3 created
```

Running `install` again is safe — existing files, correct symlinks, and an already-configured PATH entry are all skipped.

### Uninstall shell presets

```bash
shine shell uninstall                # uninstall all categories
shine shell uninstall proxy          # uninstall only the proxy category
shine shell uninstall --dry-run      # preview without changes
shine shell uninstall --purge        # also remove empty managed directories
shine shell uninstall proxy --purge  # uninstall proxy and remove its preset dir
```

Removes shine-managed symlinks from `~/.shine/bin/`, preset files from `~/.shine/presets/shell/`, and the PATH entry from your shell config. User-created files are never removed.

When a category is specified only that category's files and symlinks are removed; the PATH entry is kept so other installed categories remain usable.

`--purge` removes the target directory (the whole `~/.shine/presets/shell/` tree when no category is given, or only `~/.shine/presets/shell/<category>/` when one is specified). It never removes `~/.shine/config.toml` or the root `~/.shine/` directory.

### List available app presets

```bash
shine app list
```

```
App Preset Categories

  JetBrains  JetBrains IDEs configuration.
  git        Personal git configuration with common aliases and sensible defaults.
  starship   Starship prompt: minimal left-prompt with git branch and status.
  vim        Vim configuration directory with base config and machine-local overrides.  2 files

Run `shine app install <CATEGORY>` to install a specific category.
Run `shine app install` to install all.
```

### Show app preset details

```bash
shine app info starship
shine app info vim
```

Prints the description, destination, and file list for a single category, with per-file install status when the category has already been installed.

### Install app presets

```bash
shine app install             # install all app categories
shine app install starship    # install only one category
shine app install --dry-run   # preview destination writes
```

`shine app install` first extracts bundled files to `~/.shine/presets/app/`, then copies them to their final destinations.

```
Installing  3 files available
  ✓  gitconfig   →  ~/.gitconfig
  ✓  starship.toml  →  ~/.config/starship/starship.toml
  -  vimrc  already up to date

Done  2 installed · 1 skipped
```

If `presets/app/<CATEGORY>/shine.toml` exists, that category uses directory-level metadata:

```toml
description = "Vim configuration directory"
dest = "~/.vim"
```

When `shine.toml` defines `files`, only those entries are installed. When it omits `files`, `shine` treats the whole category directory as managed and maps every file except `shine.toml` into `dest` with the same relative path.

#### File transforms

A `[[files]]` entry may declare a `transform` to process the source file before it is written to the destination. Use `target` to rename the file at the destination if the transform changes the format:

```toml
description = "Docker daemon configuration"
dest = "/etc/docker"

[[files]]
source      = "daemon.jsonc"
target      = "daemon.json"
description = "Docker daemon options"
transform   = "jsonc-to-json"
```

`shine install` output shows the transform step:

```
  ✓  daemon.jsonc  [jsonc-to-json]  →  /etc/docker/daemon.json
```

`shine check` compares the **transformed** output against the installed file — a source change that produces identical JSON output is reported as **up-to-date**.

**Supported transforms**

| Name | From | To | Description |
|---|---|---|---|
| `jsonc-to-json` | `.jsonc` | `.json` | Strip `//` and `/* */` comments, trailing commas; emit canonical JSON |

For a pipeline of transforms, use the `transforms` array instead:

```toml
transforms = ["jsonc-to-json"]
```

If no `shine.toml` exists, `shine` falls back to the legacy file-level rules: a preset file may start with a `shine-dest:` annotation for an explicit absolute target after `~` expansion. Without that annotation, `shine` installs to:

```text
<app_default_dest_root>/<CATEGORY>/<FILE>
```

The default `app_default_dest_root` is `~/.config`.

If the destination already exists and is not managed by `shine`, it is moved aside to `*.shine.bak` before the preset is installed. Managed app installs are tracked in `~/.shine/app-manifest.toml`, so repeat installs can safely skip unchanged files and overwrite only files previously installed by `shine`.

### Uninstall app presets

```bash
shine app uninstall                # uninstall all app categories
shine app uninstall starship       # uninstall only the starship category
shine app uninstall --dry-run      # preview without changes
shine app uninstall --purge        # also remove presets and manifest
shine app uninstall git --purge    # uninstall git category and remove its preset dir
```

Uninstall removes only app files whose content still matches the version recorded in `~/.shine/app-manifest.toml`. If a file was modified after installation, `shine` leaves it in place and reports it as user-modified. When an unmanaged file was backed up during install, uninstall restores that backup automatically.

When a category is specified only that category's managed files are removed; other installed categories are unaffected.

`--purge` additionally removes `~/.shine/presets/app/<category>/` when a category is given, or the full `~/.shine/presets/app/` and `~/.shine/app-manifest.toml` when no category is given.

### List installed presets and configs

```bash
shine list
```

Shows only items that are currently installed or configured — a quick "what's set up on this machine" view. Unlike `shine check`, entries that are not installed are omitted.

```
Shell Presets
  ✓  proxy/setproxy       installed
  ✓  proxy/usetproxy      installed

App Configs
  ✓  git      →  ~/.gitconfig                    up-to-date
  ↑  starship →  ~/.config/starship/starship.toml  update available

Summary  2 shell · 2 app
```

If nothing is installed yet, `shine list` prints a hint to run `shine shell install` or `shine app install`.

### Check configuration status

```bash
shine check           # check both shell presets and app configs
shine check shell     # shell presets only
shine check app       # app configs only
```

Shows the status of every managed preset and config file in one view:

```
Shell Presets
  ✓  proxy/setproxy       installed
  ✓  proxy/usetproxy      installed
  ✗  tools/test_tools     not installed
  ✓  PATH configured      ~/.zshrc

App Configs
  ✓  JetBrains/IdeaVim  →  ~/.ideavimrc              up-to-date
  ✓  git                →  ~/.gitconfig               up-to-date
  ↑  starship           →  ~/.config/starship/...     update available  run `shine app install`
  ✗  vim                →  ~/.vim                     not installed

Summary  2 up-to-date · 1 update available · 1 not installed
```

Status symbols:

| Symbol | Meaning |
|--------|---------|
| `✓` | Installed and up-to-date |
| `↑` | Update available — run `shine app install` |
| `~` | User-modified or partial install |
| `!` | Destination missing (was installed) |
| `✗` | Not installed |

### Export and customize presets

```bash
shine presets export
```

Copies all built-in shell scripts and app configs into your configured `presets_dir` (default `~/.shine/presets/`). Once exported you can edit the files freely — `shine` will read from the filesystem copy instead of the embedded binary on subsequent installs.

To use a custom directory as your preset source, set `presets_dir` in `~/.shine/config.toml`:

```toml
presets_dir = "~/dotfiles/shine-presets"
```

Then export the defaults there as a starting point:

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine presets export
```

All `install`, `check`, and `list` commands will automatically read from the external directory when `presets_dir` is configured. The active preset source is printed in each command's output so you always know which files are being used.

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
SHINE_VERSION=0.5.0 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

## Bundled Presets

### shell/proxy — `setproxy` / `usetproxy`

One-command proxy management for the entire development environment.

**Set proxy:**

```bash
source setproxy           # auto-detect SOCKS5 or fall back to HTTP
source setproxy sock5     # force SOCKS5
source setproxy http      # force HTTP
```

Configures simultaneously:
- Shell environment variables (`http_proxy`, `https_proxy`, `all_proxy`, …)
- Git global config (`http.proxy`, `https.proxy`)
- npm / yarn / pnpm proxy settings

Default ports: HTTP `6152`, SOCKS5 `6153` (edit `~/.shine/env.toml` to change).

**Unset proxy:**

```bash
source usetproxy
```

Clears all proxy environment variables and removes git/npm/yarn/pnpm proxy config.

### shell/tools — `test_tools`

Verifies that shine-installed shell tools are callable from the current environment.

### Shell preset metadata

Shell preset categories may optionally define `presets/shell/<category>/shine.toml` to control installed command names:

```toml
description = "Proxy helper commands"

[[files]]
source = "set_proxy.sh"
target = "setproxy"

[[files]]
source = "uset_proxy.sh"
target = "usetproxy"
```

`source` points at the script file stored under the category directory. `target` controls the command name linked into `~/.shine/bin/`. When `target` is omitted, shine falls back to the script stem.

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

You can also change the fallback install root for app presets that do not carry a `shine-dest:` annotation:

```toml
app_default_dest_root = "~/.config"
```

## Directory Layout

```
~/.shine/
├── app-manifest.toml
├── config.toml
├── bin/
│   ├── setproxy         # symlink → presets/shell/proxy/set_proxy.sh
│   ├── usetproxy        # symlink → presets/shell/proxy/uset_proxy.sh
│   └── test_tools       # symlink → presets/shell/tools/test_tools.sh
└── presets/
    ├── app/
    │   ├── JetBrains/
    │   │   └── .ideavimrc
    │   ├── git/
    │   │   └── gitconfig
    │   └── starship/
    │       └── starship.toml
    └── shell/
        ├── proxy/
        │   ├── shine.toml
        │   ├── set_proxy.sh
        │   └── uset_proxy.sh
        └── tools/
            └── test_tools.sh
```

Installed app files live at their annotated destinations, for example:

```text
~/.gitconfig
~/.ideavimrc
~/.config/starship/starship.toml
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
│       ├── colors.rs          # TTY-aware color helpers (degrades gracefully with NO_COLOR)
│       ├── presets.rs         # embedded-asset extraction, list_categories
│       ├── apps/              # app preset install/uninstall, manifest, destination resolution
│       ├── config/            # Config struct, load/save, env-var priority chain
│       ├── commands/          # clap subcommand definitions
│       └── shells/            # ShellType, install/uninstall/list, PATH injection
├── utils/      # library crate — TOML comment-preserving migration
└── presets/    # bundled shell/app files embedded into the binary at compile time
    ├── app/
    └── shell/
```

## License

MIT OR Apache-2.0
