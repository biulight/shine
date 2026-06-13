# shine

A Rust CLI for managing shell presets, app configs, and system bootstrap presets.

`shine` bundles reusable shell scripts, app configuration presets, and OS bootstrap presets into a single binary. It installs managed assets under `~/.shine/`, links shell commands into `~/.shine/bin/`, and can also copy app config files to their final destinations.

中文文档: [`docs/README.zh-CN.md`](docs/README.zh-CN.md)

## Features

- **Embedded presets** — shell scripts and app configs are compiled into the binary; no internet required after installation
- **External presets** — point `presets_dir` at your own directory (e.g. a dotfiles repo) and `shine` reads from there instead; `shine export` seeds it with the built-ins
- **Project-local presets** — run `shine init` inside a presets repo to create a local `shine.config.toml` that points `presets_dir` at the repo
- **Managed bin directory** — `~/.shine/bin/` holds flat symlinks on Unix and command shims on Windows
- **Auto PATH setup** — `install` appends `~/.shine/bin` to your shell config automatically
- **Category install/uninstall** — install or uninstall all presets or a specific subset (e.g. `proxy`)
- **Installed-only view** — `shine list` shows installed items without status noise
- **Safe uninstall** — removes only shine-managed files; user-created files are never touched
- **Dry-run support** — preview any destructive operation before it runs
- **TOML config** — `~/.shine/config.toml` with comment preservation on updates
- **App preset installer** — install managed config files like `~/.gitconfig`, `~/.config/starship/starship.toml`, or `~/.config/ghostty/config.ghostty`
- **Installed content inspection** — `shine info <target>` prints metadata, colorized status, and expected-content diffs for installed app configs and shell presets; add `--verbose` for full content
- **Release update check** — checks GitHub Releases at runtime with a 24h cache
- **Multi-shell support** — bash, zsh, and PowerShell, with per-platform shell preset entries when a category needs different files on Unix and Windows
- **System init presets** — bootstrap the current OS with curated setup steps via `shine sys init`

Current support scope: `shine shell` supports bash, zsh, and PowerShell. Windows support covers `shine self`, `shine shell`, selected app presets such as `docker-engine` and `docker-desktop`, and a Windows `shine sys init` preset implemented with PowerShell.

## Planning Workflow

Repository planning is managed in GitHub with a lightweight issue-based flow:

- Open ideas with the `Idea / Plan` issue template
- Promote accepted work into `Task` issues
- Track state with `status:` labels
- Use milestones only for release-relevant work

The full workflow lives in [`docs/PLAN.md`](docs/PLAN.md).

## Release Branch Workflow

- `release` is the primary integration and release branch.
- Regular pushes and feature PRs should target `release`.
- Version tags (`v*`) should be created from `release`; CI will build artifacts and create the GitHub Release.
- After CI creates the GitHub Release, it automatically opens a PR from `release` to `main`.
- `main` is reserved for that post-release sync PR instead of day-to-day development.

## Installation

macOS/Linux:

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

Or install from source:

```bash
cargo install --path cli
```

Windows support covers `shine self`, `shine shell`, selected app presets in PowerShell, and a PowerShell-backed `shine sys init` preset, including profile updates for both `powershell.exe` and `pwsh.exe`.

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

  agent  1 script
    ccenv         Configure Claude Code to use DeepSeek in the current shell session.
                   ...

  proxy  2 scripts
    setproxy      Set HTTP/HTTPS proxy environment variables.
                  ...
    usetproxy     Unset all proxy environment variables.
                  ...

  utils  1 script
    copyfile      Copy a file's contents to the local clipboard via OSC52.
                  ...
```

### Install shell presets

```bash
shine install proxy            # shorthand for a matching shell/app category
shine shell install            # install all categories
shine shell install proxy      # install only the proxy category
shine reinstall proxy          # shorthand reinstall for a matching category
shine shell reinstall proxy    # overwrite managed files and links for proxy
```

Extracts embedded shell scripts to `~/.shine/presets/shell/`, creates symlinks or Windows shims in `~/.shine/bin/`, and appends a PATH entry to your shell config (`~/.zshrc`, `~/.bashrc`, PowerShell profile, etc.):

```
Shell Presets  4 created
Bin Links      4 created
```

Installing all shell presets includes `agent`, which requires `DEEPSEEK_API_KEY` or `DEEPSEEK_API_KEY_GPG_SECRET` in the active env config before use.
Running `install` again is safe — existing files, correct symlinks, and an already-configured PATH entry are all skipped. Use `reinstall` when you want to overwrite managed preset files, links, and the shell config entry.

Top-level `install`, `reinstall`, and `uninstall` commands accept a required category and automatically route to either `shell/<category>` or `app/<category>`. If both preset types define the same category name, `shine` prompts you to choose one.

Shell metadata can scope entries to `platforms = ["unix"]` or `platforms = ["windows"]`. The built-in `agent` category uses this to expose `ccenv` from `cc.sh` on Unix shells and from `cc.ps1` on Windows PowerShell.

On Windows, PowerShell PATH setup updates both supported profile locations so `powershell.exe` and `pwsh.exe` see the same `~/.shine/bin` entry:

- `~/Documents/PowerShell/Microsoft.PowerShell_profile.ps1`
- `~/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1`

### Uninstall shell presets

```bash
shine uninstall proxy              # shorthand for a matching shell/app category
shine shell uninstall                # uninstall all categories
shine shell uninstall proxy          # uninstall only the proxy category
shine shell uninstall --dry-run      # preview without changes
shine shell uninstall --purge        # also remove empty managed directories
shine shell uninstall proxy --purge  # uninstall proxy and remove its preset dir
```

Removes shine-managed symlinks or shims from `~/.shine/bin/`, preset files from `~/.shine/presets/shell/`, and the PATH entry from your shell config. User-created files are never removed.

When a category is specified only that category's files and symlinks are removed; the PATH entry is kept so other installed categories remain usable.

`--purge` removes the target directory (the whole `~/.shine/presets/shell/` tree when no category is given, or only `~/.shine/presets/shell/<category>/` when one is specified). It never removes `~/.shine/config.toml` or the root `~/.shine/` directory.

### Generate shell completions

```bash
shine completions bash > ~/.local/share/bash-completion/completions/shine
shine completions zsh > ~/.zfunc/_shine
shine completions powershell > shine-completions.ps1
```

`shine completions <shell>` prints a completion script to `stdout` for manual installation. It supports `bash`, `zsh`, and `powershell`, and it does not modify your shell config automatically.

### List available app presets

```bash
shine app list
```

```
App Preset Categories

  JetBrains  JetBrains IDEs configuration.
  ghostty    Ghostty terminal configuration.
  git        Personal git configuration with common aliases and sensible defaults.
  starship   Starship prompt: minimal left-prompt with git branch and status.
  vim        Vim configuration directory with base config and machine-local overrides.  2 files

Run `shine app install <CATEGORY>` to install a specific category.
Run `shine app install` to install all.
```

### List available system init presets

```bash
shine sys list
```

Lists the built-in OS bootstrap presets and marks the current platform with `▶`.

### Run system init for the current OS

```bash
shine sys init
shine sys init --preset recommended
shine sys init --dry-run
```

`shine sys init` detects the current OS, loads `presets/sys/<os>/shine.toml`, resolves a set of install items, and then runs the platform init script once per selected item. After all items finish, it calls the same script with `__shine_finalize` so the preset can apply shared profile or shell integration once.

- In a TTY, `shine sys init` opens an interactive multi-select with defaults taken from the preset's `default_profile`.
- `shine sys init --preset <PROFILE>` skips the prompt and applies that named profile directly.
- Without a TTY, `shine sys init` falls back to `default_profile`.
- `shine sys init --dry-run` prints the resolved items, per-item script invocations, finalize invocation, and script content without executing anything.

System init presets use this metadata shape:

```toml
description = "Initialize Ubuntu system with selectable setup steps."
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install the latest stable Neovim release."

[profiles.recommended]
items = ["neovim"]
```

Init scripts can emit a machine-readable status line so `shine` can render a compact summary:

```bash
printf 'SHINE_SYS_STATUS\t%s\t%s\n' "already-installed" "nvim found"
```

Supported states are `installed`, `already-installed`, `skipped`, `updated`, `needs-action`, `completed`, and `failed`. Other script output is preserved as indented logs for the current item. Older scripts that do not emit status lines still run; successful items are shown as `completed`.

Current built-in presets:

- `ubuntu` — offers selectable Neovim, AstroNvim, Atuin, Yazi, Starship, zoxide, zsh-vi-mode, fzf, bat, eza, pnpm, mise, Homebrew, and ZeroTier steps. The `recommended` profile includes the core editor, history, file manager, prompt, navigation, and shell utility steps while leaving pnpm, mise, Homebrew, and ZeroTier opt-in through the `all` profile or explicit selection.
- `macos` — offers selectable Homebrew, Yazi, Starship, Neovim, AstroNvim, ZeroTier, zsh plugin, zoxide, Atuin, fzf, bat, eza, nvm, Bun, pnpm, and Fastfetch steps. The `recommended` profile includes Homebrew and the core terminal/editor tools; the `all` profile adds JavaScript runtimes and Fastfetch.
- `windows` — offers selectable Rust, Yazi, Starship, zoxide, Atuin, fzf, bat, eza, ZeroTier, Bun, pnpm, and mise steps. The `recommended` profile includes Rust and core terminal tools; the `all` profile adds JavaScript runtime and environment manager steps.

When selected tools need shell integration, sys init installs managed profile blocks. Ubuntu uses a managed shell profile loader for tools such as Yazi, Starship, zoxide, Atuin, fzf, and mise. Windows uses a managed PowerShell profile loader for Yazi, Starship, zoxide, Atuin, fzf, and mise. Managed profile updates are merged into the existing profile file so user edits outside the managed block are preserved.

### Show app preset details

```bash
shine app info starship
shine app info ghostty
shine app info vim
```

Prints the description, destination, and file list for a single category, with per-file install status when the category has already been installed.

### Install app presets

```bash
shine install starship        # shorthand for a matching shell/app category
shine app install             # install all app categories
shine app install ghostty     # install only one category
shine app install starship    # install only one category
shine app install --dry-run   # preview destination writes
shine reinstall ghostty       # shorthand reinstall for a matching category
shine app reinstall ghostty   # overwrite managed files for one category
```

`shine app install` first extracts bundled files to `~/.shine/presets/app/`, then copies them to their final destinations.

```
Installing  4 files available
  ✓  config.ghostty  →  ~/.config/ghostty/config.ghostty
  ✓  gitconfig   →  ~/.gitconfig
  ✓  starship.toml  →  ~/.config/starship/starship.toml
  -  vimrc  already up to date

Done  3 installed · 1 skipped
```

If `presets/app/<CATEGORY>/shine.toml` exists, that category uses directory-level metadata:

```toml
description = "Vim configuration directory"
dest = "~/.vim"
```

When `shine.toml` defines `files`, only those entries are installed. When it omits `files`, `shine` treats the whole category directory as managed and maps every file except `shine.toml` into `dest` with the same relative path.

`dest` must expand to an absolute path for the current platform before `shine app install` writes files. Metadata can also use a platform map so one category resolves to different roots on Unix and Windows:

```toml
[dest]
windows = "~/.docker"
unix = "/etc/docker"
```

#### File transforms

A `[[files]]` entry may declare a `transforms` pipeline to process the source file before it is written to the destination. Use `target` to rename the file at the destination if a transform changes the format:

```toml
description = "Docker Engine daemon configuration"

[dest]
windows = "~/.docker"
unix = "/etc/docker"

[[files]]
source      = "daemon.jsonc"
target      = "daemon.json"
description = "Docker Engine daemon options"
transforms  = ["jsonc-to-json"]
```

`shine app install` output shows the transform step:

```
  ✓  daemon.jsonc  [jsonc-to-json]  →  ~/.docker/daemon.json
```

`shine update` compares the **transformed** output against the installed file — a source change that produces identical JSON output is reported as **up-to-date**.

For JSON settings files that should keep unrelated user values, a `[[files]]` entry can opt into managed-key merging instead of full-file replacement:

```toml
[[files]]
source = "settings-store.jsonc"
target = "settings-store.json"
transforms = ["template", "jsonc-to-json"]
install_mode = "json-merge"
managed_keys = ["proxy", "containersProxy"]
```

`json-merge` treats the transformed source as a JSON object, updates only the listed top-level keys in the destination file, and removes only those same keys on uninstall.

The bundled `docker-engine` app preset manages Docker Engine daemon config. On Windows it writes `daemon.json` to `~/.docker/daemon.json`; on Unix it writes `/etc/docker/daemon.json`. That path is the Docker Engine daemon config path, not the Docker Desktop proxy settings path.

The bundled `docker-desktop` app preset is Windows-only in v1. It merges Docker Desktop proxy settings into `~/AppData/Roaming/Docker/settings-store.json` and only manages the `proxy` and `containersProxy` keys, leaving other Docker Desktop settings untouched.

Shell scripts that opt into template substitution with `# shine-template: true` are checked the same way. `shine update` re-renders the source script with the current `[env]` values and reports `update available` when the rendered output differs from the installed script, including when the source lives in an external `presets_dir`.

**Supported transforms**

| Name | From | To | Description |
|---|---|---|---|
| `jsonc-to-json` | `.jsonc` | `.json` | Strip `//` and `/* */` comments, trailing commas; emit canonical JSON |

Use the same `transforms` array for single-step or multi-step pipelines:

```toml
transforms = ["jsonc-to-json"]
```

For backward compatibility, `transform = "jsonc-to-json"` is also accepted for a single transform, but new presets should prefer `transforms = [...]`.

If no `shine.toml` exists, `shine` falls back to the legacy file-level rules: a preset file may start with a `shine-dest:` annotation for an explicit absolute target after `~` expansion. Without that annotation, `shine` installs to:

```text
<app_default_dest_root>/<CATEGORY>/<FILE>
```

The default `app_default_dest_root` is `~/.config`.

If the destination already exists and is not managed by `shine`, it is moved aside to `*.shine.bak` before the preset is installed. Managed app installs are tracked in `~/.shine/app-manifest.toml`, so repeat installs can safely skip unchanged files and overwrite only files previously installed by `shine`.

### Uninstall app presets

```bash
shine app uninstall                # uninstall all app categories
shine app uninstall ghostty        # uninstall only the ghostty category
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

Shows only items that are currently installed or configured — a quick "what's set up on this machine" view. Entries that are not installed are omitted and status details are not shown.
Shell presets whose source file exists but whose command symlink is missing are also omitted, because they are not callable from `~/.shine/bin/`.

```
Shell Presets
  proxy/setproxy
  proxy/usetproxy

App Configs
  git       →  ~/.gitconfig
  ghostty   →  ~/.config/ghostty
  starship  →  ~/.config/starship/starship.toml
```

If nothing is installed yet, `shine list` prints a hint to run `shine shell install` or `shine app install`.

### Inspect installed config details

```bash
shine info git
shine info starship
shine info proxy
shine info setproxy
shine info git --verbose
```

Shows metadata, colorized status, and when applicable an expected-content diff for a managed app config or shell preset. Add `--verbose` to also print the installed or rendered file content. The target is matched against installed categories, command names, display names, source filenames, and destination basenames. If a short target is ambiguous, use the canonical form shown in the error:

```bash
shine info app/git
shine info shell/proxy/setproxy
```

For app configs, `shine info --verbose` reads the installed destination file. For shell presets, it reads the effective script target, including rendered template scripts under `~/.shine/rendered/` when applicable.

### Update status and release check

```bash
shine update
shine update --verbose
```

Shows only available installed configuration updates, then checks for a newer shine release. Use `--verbose` to include installed entries that are already up-to-date or need attention:

```
Shell Presets
  ↑  proxy/setproxy       update available  run `shine upgrade`

App Configs
  ↑  starship           →  ~/.config/starship/...     update available  run `shine upgrade`
```

Status symbols:

| Symbol | Meaning |
|--------|---------|
| `✓` | Installed and up-to-date |
| `↑` | Update available — run `shine upgrade` |
| `~` | User-modified or partial install |
| `!` | Destination missing (was installed) |
| `✗` | Not installed |

### Export and customize presets

```bash
shine export
```

Copies all built-in shell scripts and app configs into your configured `presets_dir` (default `~/.shine/presets/`). Once exported you can edit the files freely — `shine` will read from the filesystem copy instead of the embedded binary on subsequent installs.

To switch `shine` to a custom preset source directory with the CLI:

```bash
shine link ~/dotfiles/shine-presets --create
shine export
```

To use a custom directory as your preset source, set `presets_dir` in `~/.shine/config.toml`:

```toml
presets_dir = "~/dotfiles/shine-presets"
```

Then export the defaults there as a starting point:

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine export
```

All `install`, `update`, and `list` commands will automatically read from the external directory when `presets_dir` is configured. The active preset source is printed in each command's output so you always know which files are being used.

### Initialize a presets directory

```bash
cd ~/dotfiles/shine-presets
shine init
```

`shine init` creates `shine.config.toml` in the current directory and writes `presets_dir = "."`, so the file can be committed to Git and reused on another machine. On later runs from that directory or any child directory, `shine` finds the nearest ancestor `shine.config.toml`, resolves relative paths from that file's directory, and keeps runtime state such as `bin/`, rendered template scripts, update-check cache, and app manifests under `~/.shine/`.

The command asks for confirmation before writing. Use `shine init --yes` for scripts.

### Runtime update policy

`shine` checks the latest GitHub Release for `biulight/shine` before executing commands and caches the result for 24 hours under `~/.shine/`.

- Newer `major` or `minor` release: prints an upgrade reminder and continues
- Newer `patch` release: requires `shine self upgrade` before continuing
- Network/API failures: silently skipped, command execution continues
- Cache writes are best-effort: if `~/.shine/update-check.json` cannot be written, the update result is still used for the current command

Manual commands:

```bash
shine update        # show available updates, then force-check the latest release
shine update --verbose  # include up-to-date and non-update status rows
shine self install  # copy the current binary to the platform default install path
shine self install --dest ~/.local/bin/shine  # install to a custom path
shine self upgrade  # download and install the latest stable release for this platform
shine self upgrade --channel stable   # explicitly reinstall the stable release
shine self upgrade --channel preview  # install the moving preview prerelease
shine upgrade       # force-update installed shell and app configs
shine upgrade --verbose  # include env-template check details
```

`shine self install` defaults to `/usr/local/bin/shine` on macOS/Linux and `%LOCALAPPDATA%\Programs\shine\shine.exe` on Windows. It detects whether the install directory is on `PATH` and prints a platform-specific hint when it is not, but it does not edit `PATH` automatically.

Preview upgrades install from the fixed `preview` GitHub prerelease and are not used by automatic update checks. If the installed preview already matches the current prerelease build, `shine self upgrade --channel preview` reports it as up to date instead of reinstalling. Preview binaries identify themselves with SemVer build metadata in `shine --version`, for example `0.31.1+preview.abc1234`, while stable binaries continue to report `0.31.1`.

If the cache directory under `~/.shine/` is missing, `shine` recreates it automatically before saving the update-check cache.

### Installer options

`install.sh` defaults to installing `shine` into `~/.local/bin/shine` without editing your shell config.

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=0.31.1 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

`install.ps1` defaults to installing `shine.exe` into `%LOCALAPPDATA%\Programs\shine\shine.exe` without editing your user PATH.

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "0.31.1"; .\install.ps1
$env:SHINE_REPO = "biulight/shine"; .\install.ps1
```

## Bundled Presets

### app/ghostty

The bundled Ghostty preset installs a main `config.ghostty` plus paired light and dark themes under `~/.config/ghostty/themes/`. The default config uses automatic light/dark theme switching:

```text
theme = light:light_Github Light Default,dark:dark_Alien Blood
```

Set `GHOSTTY_BG_LIGHT` and `GHOSTTY_BG_DARK` with `shine env set` if you want the bundled light and dark themes to render a background image path during install or `shine upgrade`.

### shell/proxy — `setproxy` / `usetproxy`

One-command proxy management for the current terminal session.

**Set proxy:**

```bash
setproxy           # auto-detect SOCKS5 or fall back to HTTP
setproxy sock5     # force SOCKS5
setproxy http      # force HTTP
```

After a fresh `shine shell install proxy`, reload your shell config once (for example,
`source ~/.zshrc` or `. $PROFILE`) or open a new shell before using `setproxy` directly.

Configures simultaneously:
- Shell environment variables (`http_proxy`, `https_proxy`, `all_proxy`, …)
- npm-compatible process config (`npm_config_proxy`, `npm_config_https_proxy`) for npm and pnpm
- Git-compatible proxy environment variables

Yarn is the exception: when Yarn is installed, `setproxy` prints a notice and updates Yarn proxy config because Yarn proxy settings are not reliably scoped to the current shell.

Default ports: HTTP `6152`, SOCKS5 `6153` (edit `[env]` in `~/.shine/config.toml` to change).

**Unset proxy:**

```bash
usetproxy
```

Clears the session proxy environment variables. If Yarn is installed, it also removes the Yarn proxy config entries that `setproxy` may have written.

### shell/utils — `copyfile`

Small utility commands for terminal workflows. The built-in Unix `copyfile <file>` command copies a file's contents to the local clipboard via OSC52, which is useful over SSH or inside terminal multiplexers that support OSC52 clipboard integration.

### shell/agent — `ccenv`

Configures the current shell for Claude Code with the DeepSeek provider.

Add your key to the global env override at `~/.shine/shine.env.toml`, or to a
project-local env file next to `shine.config.toml`:

```toml
DEEPSEEK_API_KEY = "..."
```

Or store a base64-encoded GPG secret instead:

```toml
DEEPSEEK_API_KEY_GPG_SECRET = "<base64-gpg-ciphertext>"
```

Create the encrypted value with your existing GPG key. If the private key is
backed by a YubiKey, `gpg-agent` will handle PIN/touch prompts during `ccenv`:

```bash
shine env encrypt --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_GPG_SECRET
```

`shine env encrypt` uses `gpg_key_id` from `config.toml` by default. Pass
`-r/--recipient <key-id>` to override it for a single command.

You can also decrypt any base64 GPG secret from the active env config directly:

```bash
shine env decrypt DEEPSEEK_API_KEY_GPG_SECRET
```

For a value that should become an environment variable in the current shell,
store it as `<KEY>_SECRET` for encrypted storage or `<KEY>` for plaintext
fallback, then evaluate the generated shell code:

```bash
shine env encrypt --from MY_TOKEN
eval "$(shine env export MY_TOKEN)"
```

`shine env export MY_TOKEN` prefers `MY_TOKEN_SECRET`, decrypts it when present,
and otherwise falls back to `MY_TOKEN`. It prints shell-specific assignment code;
the `eval` step is what applies it to the current terminal session. Use `--set`
when you need a custom encrypted target key.

Then install and use the helper:

```bash
shine shell install agent
ccenv
```

When both `DEEPSEEK_API_KEY_GPG_SECRET` and `DEEPSEEK_API_KEY` are set, the
encrypted secret wins. A GPG decode/decrypt failure stops `ccenv` instead of
falling back to plaintext.

### Shell preset metadata

Shell preset categories may optionally define `presets/shell/<category>/shine.toml` to control installed command names:

```toml
description = "Proxy helper commands"

[[files]]
source = "set_proxy.sh"
target = "setproxy"
needs_source = true
platforms = ["unix"]

[[files]]
source = "set_proxy.ps1"
target = "setproxy"
needs_source = true
platforms = ["windows"]
```

`source` points at the script file stored under the category directory. `target` controls the command name linked into `~/.shine/bin/`. When `target` is omitted, shine falls back to the script stem. `platforms` is optional; supported values are `unix` and `windows`, and omitted means all platforms.

## Configuration

`~/.shine/config.toml` is created automatically on first run. The global config
keeps the generic `config.toml` name because `~/.shine/` is already a
shine-specific directory. Project-local preset repos can additionally use
`shine.config.toml` to avoid colliding with other tools' `config.toml` files.

Override directories at runtime:

```bash
SHINE_CONFIG_DIR=/custom/path shine shell install   # override shine dir + presets dir
SHINE_PRESETS=/custom/presets shine shell install   # override presets dir only
```

Or persist a custom presets directory in `~/.shine/config.toml`:

```toml
presets_dir = "/custom/presets"
```

Config discovery searches the current directory and its parents for `shine.config.toml`. If none is found, legacy project `config.toml` files that contain `presets_dir` are still recognized with a warning. Otherwise, `shine` uses the global config under `~/.shine/` or `SHINE_CONFIG_DIR`.

Preset source priority is: `SHINE_PRESETS` > active config `presets_dir` > default. When `SHINE_CONFIG_DIR` is set and no project config is active, it also sets the default presets directory to `$SHINE_CONFIG_DIR/presets`.

You can also change the fallback install root for app presets that do not carry a `shine-dest:` annotation:

```toml
app_default_dest_root = "~/.config"
```

Set a default GPG recipient for `shine env encrypt`:

```toml
gpg_key_id = "<key-id>"
```

Template variables live in the `[env]` table:

```toml
[env]
HTTP_PROXY_PORT = "6152"
SOCKS5_PROXY_PORT = "6153"
PROXY_HOST = "127.0.0.1"
PROXY_NO_PROXY = "localhost,127.0.0.1,::1"
GHOSTTY_BG_LIGHT = ""
GHOSTTY_BG_DARK = ""
```

Set `GHOSTTY_BG_LIGHT` and `GHOSTTY_BG_DARK` to enable appearance-specific
Ghostty wallpapers. Leaving them empty keeps the bundled Ghostty preset
installed without a background image.

For global overrides, place a flat `shine.env.toml` next to the global config at
`~/.shine/shine.env.toml`. For project-local overrides, place a flat
`shine.env.toml` next to `shine.config.toml`. Values from `shine.env.toml`
override matching keys from the active config's `[env]` table without modifying
either file. When both global and project-local env files are present, the
project-local file wins. Legacy project `.env.toml` files are still recognized
when project `shine.env.toml` is absent.

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = "127.0.0.1"
```

## Directory Layout

```
~/.shine/
├── app-manifest.toml
├── config.toml
├── shine.env.toml    # optional flat env overrides
├── bin/
│   ├── setproxy         # symlink/shim → platform proxy script
│   ├── usetproxy        # symlink/shim → platform proxy script
│   └── copyfile         # symlink → presets/shell/utils/copyfile.sh
└── presets/
    ├── app/
    │   ├── JetBrains/
    │   │   └── .ideavimrc
    │   ├── ghostty/
    │   │   ├── config.ghostty
    │   │   ├── themes/
    │   │   │   ├── Alien Blood
    │   │   │   └── Github Light Default
    │   │   └── shine.toml
    │   ├── git/
    │   │   └── gitconfig
    │   └── starship/
    │       └── starship.toml
    └── shell/
        ├── proxy/
        │   ├── shine.toml
        │   ├── set_proxy.ps1
        │   ├── set_proxy.sh
        │   ├── uset_proxy.ps1
        │   └── uset_proxy.sh
        └── utils/
            ├── shine.toml
            └── copyfile.sh
```

Installed app files live at their annotated destinations, for example:

```text
~/.gitconfig
~/.ideavimrc
~/.config/ghostty/config.ghostty
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
