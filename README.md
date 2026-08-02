# shine

A cross-platform Rust CLI for managing personal shell commands, application configs, system
resources, and repeatable machine setup.

`shine` turns dotfiles and bootstrap workflows into manifest-tracked resources that can be
installed, inspected, updated, and safely removed. It ships useful presets in one self-contained
binary, supports personal preset repositories and overlays, and applies layered environment values
without taking ownership of unrelated user files. It also provides managed system setup, encrypted
environment workflows, saved tasks, terminal-theme synchronization, and SSH session file transfer.

中文文档: [`docs/README.zh-CN.md`](docs/README.zh-CN.md)

## Features

- **Self-contained and extensible presets** — use the shell, app, and OS presets embedded in the
  binary, or link and safely pull your own Git-managed preset source and selective overlay
- **Manifest-tracked lifecycle** — install, inspect, diff, update, upgrade, and uninstall only
  resources owned by Shine, with backups, modification guards, and dry-run support where offered
- **Portable shell commands** — publish scripts or Bun programs through one managed bin directory,
  with automatic PATH and completion setup for bash, zsh, and PowerShell
- **Application configuration** — copy, transform, merge, generate, and explicitly build app
  config artifacts while preserving user-owned content
- **Layered environments and secrets** — combine global, project, and overlay values; encrypt with
  GPG or age; inject selected values into commands or remote SSH sessions
- **System setup and managed resources** — run curated OS bootstrap profiles and converge or remove
  system resources such as managed files and split DNS
- **Status and updates** — inspect installed content and expected diffs, detect preset/config drift,
  pull Git sources, and check GitHub Releases through a non-fatal 24-hour cache
- **Personal workflows** — save direct-execution tasks, synchronize terminal themes, serve managed
  local resources, and transfer files through an authenticated `shine ssh` session
- **Cross-platform operation** — macOS and Linux are the primary targets, with native Windows
  support for the CLI areas and presets described below

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
    ccenv         Launch Claude Code with a selected provider.
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

### Inspect shell preset details

Inspect a category or one of its commands before or after installation:

```bash
shine shell info proxy
shine shell info setproxy
shine shell info proxy/setproxy
```

The detail view reports source metadata, runtime requirements, transforms, declared environment
variable names, and current installation status. It never prints environment values.

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

Installing all shell presets includes `agent`. Its default Codex provider requires `CLIPROXYAPI_AUTH_TOKEN` or `CLIPROXYAPI_AUTH_TOKEN_SECRET`; DeepSeek and Qwen use their corresponding API-key variables in the active env config.
Running `install` again is safe — existing files, correct symlinks, and an already-configured PATH entry are all skipped. Use `reinstall` when you want to overwrite managed preset files, links, and the shell config entry.

Top-level `install`, `reinstall`, and `uninstall` commands accept a required category and automatically route to either `shell/<category>` or `app/<category>`. If both preset types define the same category name, `shine` prompts you to choose one.

Shell metadata can scope entries to `platforms = ["unix"]` or `platforms = ["windows"]`. The built-in `agent` category exposes one cross-platform `cc.ts` entry through the Bun runtime.

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

### Shell completions

```bash
shine completions install
```

Open a new shell, or reload your shell config once (`source ~/.zshrc` or `source ~/.bashrc`).

Installing or reinstalling a specific shell preset, such as `shine shell install proxy`, also refreshes completions as part of the managed shell profile update.

Completions are dynamic: preset categories and commands follow the active built-in, external, project, and overlay sources, while installed system-update items and saved task names come from Shine's runtime manifests. Bash, Zsh, and PowerShell are supported; on Fish or Elvish, `completions install` keeps the managed PATH setup and reports that Shine completion is unavailable.

For advanced manual setup or inspection, `shine completions <shell>` prints the registration script to `stdout` for `bash`, `zsh`, and `powershell`.

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
shine sys list --all
shine sys info split-dns
```

`shine sys list` shows every init and managed item available for the current OS, including its recorded status and the command used to enable it. Use `--all` to inspect every supported OS.

`shine sys info <ITEM>` shows an item's type, driver, administrator requirement, required environment variable names, current status, and next commands. For example, `shine sys info split-dns` explains how to enable private split DNS without exposing configured environment values.

### Run system init for the current OS

```bash
shine sys init
shine sys init --preset recommended
shine sys init --dry-run
shine sys status
shine sys update
shine sys update neovim --verbose
```

`shine sys init` detects the current OS, loads `presets/sys/<os>/shine.toml`, resolves a set of install items, and then runs the platform init script once per selected item. After successful item work, `shine` refreshes managed shell profile integration from Rust.

- In a TTY, `shine sys init` opens an interactive multi-select with defaults taken from the preset's `default_profile`.
- `shine sys init --preset <PROFILE>` skips the prompt and applies that named profile directly.
- Without a TTY, `shine sys init` falls back to `default_profile`.
- `shine sys init --dry-run` prints the resolved items, per-item script invocations, the internal profile update step, and script content without executing anything.
- `shine sys status` shows the init items previously recorded for the current OS.
- `shine sys update [ITEM] [--verbose] [--proxy]` is read-only: it checks only bootstrap software previously recorded by `shine sys init`, never installs or upgrades anything, and never changes the sys manifest or shell profile. `--proxy` routes checks through the preset proxy; on Windows it explicitly passes WinGet's `--proxy` option because WinGet ignores standard HTTP proxy environment variables. By default it shows verified package-manager updates and the exact upstream command to run. `--verbose` also shows current and manual-check-only items. Direct installers and user-owned Git configurations are intentionally reported as manual instead of guessed.

`shine update` and `shine upgrade` continue to reconcile Shine-managed configuration and managed system resources. They do not upgrade third-party bootstrap software; copying and running a command printed by `shine sys update` is always the user's explicit decision.

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
- `macos` — offers selectable Homebrew, Rust, Yazi, Starship, Neovim, AstroNvim, ZeroTier, zsh plugin, zoxide, Atuin, fzf, bat, eza, nvm, Bun, pnpm, mise, and Fastfetch steps. The `recommended` profile includes Homebrew and the core terminal/editor tools; the `all` profile adds JavaScript runtimes, mise, and Fastfetch.
- `windows` — offers selectable Rust, Yazi, Starship, zoxide, Atuin, fzf, bat, eza, ZeroTier, Bun, pnpm, and mise steps. The `recommended` profile includes Rust and core terminal tools; the `all` profile adds JavaScript runtime and environment manager steps.

When selected tools need shell integration, sys init installs managed `pre` and `post` profile loaders. The `pre` loader runs near the top of the user profile for PATH, Homebrew, and completion search path setup; the `post` loader runs near the end for Yazi, Starship, zoxide, Atuin, fzf, mise, aliases, and shell plugins. Managed profile files are merged so user edits inside them are preserved or reported for review.

On Ubuntu and macOS, the managed `pre` profile also syncs the terminal's light/dark theme via `shine theme sync`, exporting `SHINE_TERMINAL_THEME=light|dark` and setting `BAT_THEME` to `GitHub` for light backgrounds and `OneHalfDark` for dark backgrounds (override with `SHINE_BAT_LIGHT_THEME`/`SHINE_BAT_DARK_THEME`). Resolution tries, in order: an already-exported `SHINE_TERMINAL_THEME` (including the value `shine ssh` injects from your local terminal — see below), `COLORFGBG`, then a direct OSC 11 query with a total (not per-byte) read deadline. A `BAT_THEME` you've already set yourself is left untouched. Disable auto-sync with `sync_terminal_theme = false` in `config.toml` or `SHINE_SYNC_TERMINAL_THEME=0` (the env var always wins); sync manually anytime with `shine theme sync` regardless of that setting, or install the optional `shine-theme-sync` command via `shine shell install utils`. `shine ssh <host>` queries your local terminal directly before connecting, so it doesn't depend on the remote OSC query at all — see [docs/terminal-theme-sync-prd.md](docs/terminal-theme-sync-prd.md). macOS sys profile management continues to target zsh, while Ubuntu supports bash and zsh.

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

### Surge URI subscriptions

The macOS `surge` preset can turn a Base64 URI subscription into a generated
`subscription-proxies.conf`. Configure the HTTPS URL, then install the preset:

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
shine app install surge
```

This feature requires Bun. It converts compatible `ss://` and `vmess://`
records, skips VLESS and unsupported transports with a credential-free
summary, and never modifies the user-maintained `local-proxies.conf`.
The built-in generator is manual so routine `shine update` and `shine upgrade`
never consume a provider's short subscription-access window. Open that window,
then refresh only the generated file:

```bash
shine app refresh surge subscription-proxies.conf
```

`shine app refresh surge` refreshes every installed generated file in the
category. A failed refresh keeps the last-known-good managed file; a
user-modified destination is preserved unless `--force` is supplied. A
successful change reloads Surge through the preset's existing post-upgrade
hook.

`local-proxy-groups.conf` declares:

```ini
Subscription = select, DIRECT, policy-path=subscription-proxies.conf, external-policy-name-prefix="SUB · "
```

Another group can import all generated nodes with
`include-other-group=Subscription`. The active Surge profile must include
`local-proxy-groups.conf` in `[Proxy Group]`. Configure the profile and apply
the built-in, idempotent artifact once:

```bash
shine env set SURGE_PROFILE '~/Library/Application Support/Surge/Profiles/MyProfile.conf'
shine app build surge
```

The preset also installs commented, inert examples under `rules/` for three
traffic classes: `LAN Network`, `LAN PROXY`, and `Other Direct`. Each class in
`local-rules.conf` shows three alternative `RULE-SET` sources:

```ini
# RULE-SET,rules/lan.list,LAN Network
# RULE-SET,http://127.0.0.1:8080/rules/lan.list,LAN Network,update-interval=86400
# RULE-SET,https://rules.example.com/surge/lan.list,LAN Network,update-interval=86400
```

Use exactly one form per class. The relative file is recommended because Shine
already installs it beside the profile. A loopback URL requires a separate HTTP
server on the same device as Surge (`localhost` on iOS is the iOS device), while
the HTTPS form requires replacing the example host with your own server. The
example proxy, policy groups, and list entries remain commented until explicitly
enabled.

`shine app unbuild surge` removes those local section includes. App uninstall
also attempts the same teardown before removing managed files. Build and
unbuild require Bun and never run implicitly during install or upgrade.

### Clash Verge Rev rule-provider examples

The built-in `clash-verge` preset uses the same three traffic classes and ships
an inert, fully commented `merge.yaml`. Its `rule-providers` section demonstrates
three mutually exclusive source layouts:

- `type: file` for rule lists already copied under mihomo's `HomeDir`;
- `type: http` with `http://127.0.0.1:8080/...` for a separate loopback server;
- `type: http` with `https://rules.example.com/...` for a remote server.

Choose one complete provider block and uncomment the matching `LAN Network`,
`LAN PROXY`, and `Other Direct` groups and `prepend-rules`. Mihomo restricts a
file provider's `path` to its `HomeDir` unless `SAFE_PATHS` is configured, so
Shine does not automatically point it at `~/.shine` or copy files into CVR's
private data directory. As with Surge, localhost means the device running the
client, not another LAN host. The HTTP examples use `proxy: DIRECT` only for
provider downloads, preventing a loopback or private rule server from following
the selected `GLOBAL`/proxy policy; remove or change it if a remote rule server
is reachable only through a proxy. For a private provider hostname that the OS
resolves through split DNS (for example, Windows NRPT), also adapt the example
`dns.nameserver-policy`, because mihomo may use its own DNS resolver.

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

System Configs
  Private split DNS  (split-dns)
```

Managed system configs are read from the current OS entries recorded in `sys-manifest.toml`;
status details remain available through `shine sys status` and `shine sys info <ITEM>`.

If nothing is installed yet, `shine list` also points to `shine sys list` alongside the shell and
app install commands.

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
shine info sys/split-dns
```

For app configs, `shine info --verbose` reads the installed destination file. For shell presets, it reads the effective script target, including rendered template scripts under `~/.shine/rendered/` when applicable. System items require the explicit `sys/<ITEM>` form and do not accept `--diff` or `--verbose`.

### Update status and release check

```bash
shine update
shine update --diff
shine update proxy/setproxy
shine update --verbose
```

Shows only available installed configuration updates, then checks for a newer shine release. Use `--verbose` to include installed entries that are already up-to-date or need attention:

Add `--diff` to print the expected-content diff directly below each available shell or app update. Pass an installed shell/app target to inspect only that target; target mode implies diff, skips the shine release check, and can be combined with `--pull` but not `--verbose` or `--refresh-release`. Use `shine info <TARGET> --verbose` when the complete current content is needed. Managed system resources already show structured field changes and do not produce content diffs.

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

When a newer Shine runtime schema requires cleanup, inspect and apply its versioned migration with:

```bash
shine state migrate --dry-run
shine state migrate
```

### Manage and customize preset sources

```bash
shine preset export
```

Copies all built-in shell scripts and app configs into your configured `presets_dir` (default `~/.shine/presets/`). Once exported you can edit the files freely — `shine` will read from the filesystem copy instead of the embedded binary on subsequent installs.

To use one built-in preset as the starting point for an overlay, copy its complete snapshot into
the current directory using its canonical `kind/name`:

```bash
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
shine preset copy app/clash-verge
```

This creates `app/surge/` and `app/clash-verge/` with all files shipped by the current binary.
Delete files you do not intend to customize: overlay matching is per path, so removed files fall
back to the built-in version and continue receiving Shine updates. Existing files are preserved by
default; use `--force` to overwrite them. Activate the directory with `shine preset overlay link .`.

To switch `shine` to a custom preset source directory with the CLI:

```bash
shine preset link ~/dotfiles/shine-presets --create
shine preset export
```

To use a custom directory as your preset source, set `presets_dir` in `~/.shine/config.toml`:

```toml
presets_dir = "~/dotfiles/shine-presets"
```

Then export the defaults there as a starting point:

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine preset export
```

All `install`, `update`, and `list` commands will automatically read from the external directory when `presets_dir` is configured. The active preset source is printed in each command's output so you always know which files are being used.

For smaller customizations, use a presets overlay. Overlay files are merged over the active presets source—embedded or external—by matching the same relative paths, such as `app/starship/starship.toml` or `shell/proxy/set_proxy.sh`. Matching overlay files take priority, and overlay-only categories are added to the base source.

```bash
shine preset overlay link ~/dotfiles/shine-overlay --create
shine preset overlay info
shine preset overlay unlink
```

If you keep your overlay in a Git repository, you can let Shine manage the checkout for you instead of cloning it on every machine. Point the overlay at a Git URL and Shine clones it (`--depth 1`, no history) under `~/.shine/overlay` and keeps it mirrored to the remote tip:

```bash
shine preset overlay link --git https://github.com/you/shine-overlay.git   # optionally: --branch main
shine preset overlay info      # shows the URL, branch, managed path, and clone status
shine preset pull              # clones on first run, then force-mirrors to the latest commit
```

You can also just add the URL directly to `~/.shine/config.toml` and run `shine preset pull`:

```toml
presets_overlay_git = "https://github.com/you/shine-overlay.git"
# presets_overlay_git_branch = "main"   # optional; defaults to the remote's default branch
```

This is ideal when one machine maintains the overlay and the rest only consume it: each device
just needs the URL, never a manual `git clone`. Because the managed checkout is a read-only mirror,
`shine preset pull` always resets it to match the remote (surviving rebases and force-pushes) and discards
any local edits. If a pull fails (e.g. the remote is unreachable), the previous checkout is left
intact and stays in use. A manually linked `preset overlay link <path>` takes precedence over a Git URL;
the two are mutually exclusive.

When the active preset source or a manually linked overlay is managed by Git, Shine also safely fast-forwards it:

```bash
shine preset pull             # sync managed overlay + fast-forward preset/overlay repositories
shine update --pull    # pull first, then reload configuration and check status
shine upgrade --pull   # pull first, then reload configuration and apply presets
```

Pull refuses dirty worktrees and uses `git pull --ff-only`; it never stashes, rebases, resets, or
resolves conflicts. Non-Git sources are skipped, and sources inside the same repository are pulled
only once.

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
shine update --diff # include content diffs for available shell/app updates
shine update proxy/setproxy  # inspect one installed target and skip the release check
shine update --verbose  # include up-to-date and non-update status rows
shine update --pull  # pull Git-managed presets before checking status
shine self install  # copy the current binary to the platform default install path
shine self install --dest ~/.local/bin/shine  # install to a custom path
shine self upgrade  # download and install the latest stable release for this platform
shine self upgrade --channel stable   # explicitly reinstall the stable release
shine self upgrade --channel preview  # install the moving preview prerelease
shine upgrade       # force-update installed shell and app configs
shine upgrade --pull  # pull Git-managed presets before applying configs
shine upgrade --verbose  # include env-template checks and skipped/current rows
```

`shine self install` defaults to `/usr/local/bin/shine` on macOS/Linux and `%LOCALAPPDATA%\Programs\shine\shine.exe` on Windows. It detects whether the install directory is on `PATH` and prints a platform-specific hint when it is not, but it does not edit `PATH` automatically.

Preview upgrades install from the fixed `preview` GitHub prerelease and are not used by automatic update checks. If the installed preview already matches the current prerelease build, `shine self upgrade --channel preview` reports it as up to date instead of reinstalling. `shine --version` uses the same provenance layout as Cargo: stable builds report `shine 1.0.0 (<commit> <date>)`, while preview builds report `shine 1.0.0-preview (<commit> <date>)`.

If the cache directory under `~/.shine/` is missing, `shine` recreates it automatically before saving the update-check cache.

### Installer options

`install.sh` defaults to installing `shine` into `~/.local/bin/shine` without editing your shell config.

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=1.0.0 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

`install.ps1` defaults to installing `shine.exe` into `%LOCALAPPDATA%\Programs\shine\shine.exe` without editing your user PATH.

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "1.0.0"; .\install.ps1
$env:SHINE_REPO = "biulight/shine"; .\install.ps1
```

### Personal tasks

Save frequently used commands as argv-based personal tasks. Tasks normally run from the directory
where you invoke them; use `--cwd` to bind a task to an existing directory so it can be launched
reliably from anywhere:

```bash
shine task save check -- cargo test
shine task save build --cwd ~/work/project -- cargo build --release
shine task run build
shine run build                 # shorthand for `shine task run build`
shine task info build
```

`--cwd` expands `~` and resolves relative paths when the task is saved. Commands are executed
directly without an implicit shell; save an explicit `sh -c '...'` when shell syntax is required.

### SSH session file transfer

`shine ssh` opens a normal interactive SSH session (it wraps the system `ssh` binary and reuses your `~/.ssh/config`) while also establishing a session-scoped transfer channel back to the machine you launched it from. `shine local download`/`upload`/`status` then use that channel from inside the session — no separate `scp`/`rsync` invocation needed.

Selected values from the active local Shine environment can also be inherited by the remote login shell or command. Put Shine's options before the SSH destination; `KEY=ALIAS` renames the variable on the remote side:

```bash
shine ssh --with API_URL dev
shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'
shine ssh --with-secret API_TOKEN dev
```

`--with` reads only the exact plaintext `[env]` key and never decrypts `KEY_SECRET`. Decrypted values require the explicit `--with-secret KEY[=ALIAS]` form. Explicit values replace the remote process's inherited values, although remote login startup files may subsequently assign the same names again. Forwarded values are session-only and are not written to remote config files. Secrets become visible to the remote host and may also be readable from process arguments/environments by sufficiently privileged or same-user processes on either machine.

For a Windows OpenSSH remote, opt in explicitly to its PowerShell wrapper. It safely sends the
session hint, terminal theme, and selected values through an encoded command, preferring
PowerShell 7 (`pwsh.exe`) and falling back to Windows PowerShell 5.1 (`powershell.exe`), rather than
trying to run the POSIX `env ... sh -c` wrapper through `cmd.exe`:

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN intel.mac.local
```

Interactive Windows sessions load the selected PowerShell's normal profile, including Shine's
managed PATH and source-command wrappers such as `setproxy`. An explicit remote command remains a
no-profile invocation.

This mode supports SSH environment injection only. It does not create a transfer tunnel, so
`shine local download`, `upload`, and `status` are unavailable in that Windows-remote session.

```bash
cd ~/work/frontend
shine ssh dev                     # opens the session; ~/work/frontend becomes this
                                   # session's "local directory" for the commands below

# once connected, on the remote host:
shine local download result.log              # remote ./result.log -> local ~/work/frontend/result.log
shine local download output/ '~/Downloads/build/'  # directories transfer too (tar-streamed)
shine local upload notes.txt                  # local ~/work/frontend/notes.txt -> remote .
shine local upload assets/ ./public/assets/
shine local status                            # session id, connection state, local directory
```

Source/destination arguments are resolved by whichever side owns them: the first `download` argument and the second `upload` argument are always remote paths, resolved against the remote shell's current directory; the other side is always resolved against the session's local directory (the directory `shine ssh` was launched from, regardless of any `cd` after connecting). Quote a path (e.g. `'~/Downloads/'`) when you want the *other* side to expand `~`, since your local shell would otherwise expand it before `shine` sees it.

Both commands default to writing into the destination side's working directory under the source's file name, refuse to overwrite an existing destination unless `--force` is passed, and support `--dry-run` to preview the transfer without copying data. Progress is printed as a single overwritten line when attached to a terminal; piped/non-interactive runs get one final line instead. `shine local status` also works as a liveness check for the session when nothing is transferring.

The local side of `shine local` (the machine you ran `shine ssh` from) also works on Windows; its
transfer protocol still requires a POSIX (Linux/macOS) remote host.

## Bundled Presets

### app/ghostty

The bundled Ghostty preset installs a main `config.ghostty` plus paired light and dark themes under `~/.config/ghostty/themes/`. The default config uses automatic light/dark theme switching:

```text
theme = light:Shine Light,dark:dark_Alien Blood
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

### shell/utils — `copyfile` / `shine-env-export`

Small utility commands for terminal workflows. The built-in Unix `copyfile <file>` command copies a file's contents to the local clipboard via OSC52, which is useful over SSH or inside terminal multiplexers that support OSC52 clipboard integration.

Install the cross-shell env helper with `shine shell install utils`, then load a Shine env value into the current shell without writing `eval` or `Invoke-Expression` manually:

```bash
shine-env-export MY_TOKEN
shine-env-export MY_TOKEN --as API_TOKEN
```

The helper prefers `MY_TOKEN_SECRET`, decrypts it when present, and otherwise falls back to plaintext `MY_TOKEN`. `--as API_TOKEN` changes only the exported shell variable name.

### shell/agent — `ccenv`

Launches Claude Code with Codex through CLIProxyAPI (the default), DeepSeek, or Qwen.
The selected environment is scoped to the Claude process and does not modify the current shell.
The prompt lists `codex`, `deepseek`, `qwen`, and the not-yet-configured `glm5`
as options 1–4; pressing Enter selects Codex.

For the default Codex provider, add the client token from CLIProxyAPI's `api-keys`
to the global env override at `~/.shine/shine.env.toml`, or to a project-local
env file next to `shine.config.toml`:

```toml
CLIPROXYAPI_AUTH_TOKEN = "..."
# Or: CLIPROXYAPI_AUTH_TOKEN_SECRET = "<tagged-or-GPG-ciphertext>"
```

Create the encrypted value with:

```bash
shine env encrypt --from CLIPROXYAPI_AUTH_TOKEN
```

Codex connects to `http://127.0.0.1:8317` and maps Claude Code's Opus, Sonnet,
and Haiku tiers to `gpt-5.6-sol`, `gpt-5.6-terra`, and `gpt-5.6-luna`
respectively. It also sets `CLAUDE_CODE_EFFORT_LEVEL=high`, which CLIProxyAPI
passes through as Codex `reasoning.effort=high`. Configure CLIProxyAPI to use
the same token and bind it to loopback so the service is not exposed to the
local network:

```yaml
host: "127.0.0.1"
port: 8317
api-keys:
  - "..."
```

For DeepSeek, use the same plaintext-or-encrypted credential pattern:

```toml
DEEPSEEK_API_KEY = "..."
# Or: DEEPSEEK_API_KEY_SECRET = "<tagged-or-GPG-ciphertext>"
```

Create encrypted values with your existing GPG key. If the private key is
backed by a YubiKey, `gpg-agent` will handle PIN/touch prompts during `ccenv`:

```bash
shine env encrypt --from DEEPSEEK_API_KEY
```

`shine env encrypt` uses `gpg_key_id` from `config.toml` by default. Pass
`-r/--recipient <key-id>` to override it for a single command.

You can also decrypt any base64 GPG secret from the active env config directly:

```bash
shine env decrypt DEEPSEEK_API_KEY_SECRET
```

For Qwen through Alibaba Cloud's Anthropic-compatible endpoint, use the same
credential pattern with `QWEN_API_KEY`:

```toml
QWEN_API_KEY = "..."
# Or: QWEN_API_KEY_SECRET = "<tagged-or-GPG-ciphertext>"
```

Create its encrypted value with:

```bash
shine env encrypt --from QWEN_API_KEY
```

When `ccenv` prompts for a provider, choose `qwen` (or option `3`). The Claude
process receives the Alibaba Cloud endpoint, Qwen model mapping, and the
`983616` context-token limit.

#### age + Apple Touch ID (Secure Enclave)

For secrets that need to be shared through a repo and decrypted by every teammate — not just
GPG users — `shine env encrypt`/`decrypt`/`seal` also support
[age](https://github.com/FiloSottile/age) as a second backend, with optional Touch ID support on
macOS via [age-plugin-se](https://github.com/remko/age-plugin-se):

```bash
brew install age age-plugin-se   # or your package manager of choice

# Generate a Secure Enclave identity that prompts Touch ID on decrypt
shine env identity init --touch-id

# Or a plain identity that works on any OS
shine env identity init
```

`identity init` prints the identity's `age1...`/`age1se1...` recipient. Add every teammate's
recipient (their own age or Secure Enclave identity) to `age_recipients` so any of them can
decrypt what you seal:

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
```

```bash
shine env encrypt --backend age --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_SECRET
shine env decrypt DEEPSEEK_API_KEY_SECRET   # prompts Touch ID if the identity is Secure Enclave
```

Ciphertext produced by the age backend is tagged (`age:...`) so `shine` always knows which
backend to decrypt with — existing untagged GPG secrets keep working unmodified. `-r/--recipient`
is repeatable for both backends, so a single `encrypt`/`seal` can target several recipients at
once. Removing a recipient from `age_recipients` does not retroactively revoke access to secrets
already committed to git history — re-seal to rotate.

For a value that should become an environment variable in the current shell,
store it as `<KEY>_SECRET` for encrypted storage or `<KEY>` for plaintext
fallback, then evaluate the generated shell code:

```bash
shine env encrypt --from MY_TOKEN
eval "$(shine env export MY_TOKEN)"
eval "$(shine env export MY_TOKEN --as API_TOKEN)"
```

`shine env export MY_TOKEN` prefers `MY_TOKEN_SECRET`, decrypts it when present,
and otherwise falls back to `MY_TOKEN`. It prints shell-specific assignment code;
the `eval` step is what applies it to the current terminal session. Pass `--as
API_TOKEN` to use a different variable name in the shell, or install the `utils`
preset and run `shine-env-export MY_TOKEN --as API_TOKEN` to apply it directly.
Use `--set` when you need a custom encrypted target key.

To expose values only to one child process without changing the current shell,
use the repeatable `--with` option on `env run`:

```bash
shine env run --with MY_TOKEN -- bun run build
shine env run --with MY_TOKEN=API_TOKEN -- bun run build
shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build
```

Each value follows the same encrypted-first lookup as `env export`. The optional
name after `=` is the environment variable visible to the child process. Explicit
`--with` values override variables inherited from the shell and values loaded from
a workspace, and no workspace file is required when at least one `--with` is used.

### Workspace environment runner

For projects that should not keep plaintext dotenv files, add a
`shine.workspace.toml`:

```toml
version = 1

[env]
modes = ["development", "production"]
default_mode = "development"
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
recipient = "alice@example.com"
```

Each environment source may mix plaintext and encrypted values:

```toml
version = 1

[plain]
VITE_APP_NAME = "My App"

[secret]
DATABASE_URL = true        # keep the existing encrypted value
API_TOKEN = false          # prompt securely on the next seal
SENTRY_TOKEN = "new-value" # seal this value, then replace it with true

[payload]
data = "<managed GPG ciphertext>"
```

Seal pending values, then run a command with the merged environment:

```bash
shine env seal
shine env run --mode production -- bun run build
```

Sources are merged in the configured order, with later files winning. Existing
process variables win by default; set `env.override_process_env = true` to let
workspace values replace them. Explicit `--with` values always win when combined
with a workspace. `env run` automatically maintains an encrypted,
mode-specific cache in the operating system cache directory. The cache is an
implementation detail and is rebuilt whenever the workspace, source contents,
or layer order changes.

Add local source files to `.gitignore` when they contain personal overrides:

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

Then install and use the helper:

```bash
shine shell install agent
ccenv
```

Running `ccenv` selects a provider and starts interactive Claude. Claude Code arguments are
forwarded unchanged; `-r`/`--run` remain compatibility aliases for an argument-free launch:

```bash
ccenv --run
ccenv --print "hello"
```

The `-r`/`--run` flag is recognized only as the first argument. Use `--` when a conflicting
argument must be passed through to Claude itself:

```bash
ccenv -- --run
```

Credentials resolve in this order: `KEY_SECRET`, legacy `KEY_GPG_SECRET`, then plaintext `KEY`.
Any selected secret's decode/decrypt failure stops `ccenv` instead of falling back.

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

Config discovery searches the current directory and its parents for `shine.config.toml`. Generic
project `config.toml` files are ignored. A project config is a sparse override layer on top of the
global config under `~/.shine/` or `SHINE_CONFIG_DIR`: fields omitted by the project inherit their
global values, while fields explicitly present in the project take priority. Relative paths are
resolved from the directory containing the file that defines them. Saving a project setting does
not copy inherited global values into the project file.

Preset source priority is: `SHINE_PRESETS` > project `presets_dir` > global `presets_dir` > default. `SHINE_CONFIG_DIR` selects the global config and runtime-state directory; its default presets directory is `$SHINE_CONFIG_DIR/presets`.

You can also change the fallback install root for app presets that do not carry a `shine-dest:` annotation:

```toml
app_default_dest_root = "~/.config"
```

Set a default GPG recipient for `shine env encrypt`:

```toml
gpg_key_id = "<key-id>"
```

Or make age the default backend and configure its recipients/identity (see
[age + Apple Touch ID](#age--apple-touch-id-secure-enclave) above):

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"   # optional; this is also the default path
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

Environment values merge by key in this order: built-in defaults, global `[env]`, project `[env]`, global `shine.env.toml`, active presets-overlay `shine.env.toml`, then project `shine.env.toml`.

`shine env list` displays these values with descriptions from the active preset
catalog and redacts sensitive values by default. Use `--reveal` when the full
value is required. A value can carry a config-local description without
separating it from its key:

```toml
[env]
MY_API_TOKEN = { value = "secret", description = "Internal API access token" }
```

Preset authors can provide shared metadata in `<presets>/env.toml`:

```toml
[[variables]]
key = "MY_API_TOKEN"
description = "Internal API access token"
sensitive = true
```

An inline description takes precedence over the preset catalog. Catalog
metadata never stores or supplies the variable value.

Set `GHOSTTY_BG_LIGHT` and `GHOSTTY_BG_DARK` to enable appearance-specific
Ghostty wallpapers. Leaving them empty keeps the bundled Ghostty preset
installed without a background image.

For global overrides, place a flat `shine.env.toml` next to the global config at
`~/.shine/shine.env.toml`. For project-local overrides, place a flat
`shine.env.toml` next to `shine.config.toml`. Values from `shine.env.toml`
override matching keys from the active config's `[env]` table without modifying
either file. When both global and project-local env files are present, the
project-local file wins. Generic project `.env.toml` files are ignored.

As of v0.40, the former global `~/.shine/env.toml` is no longer migrated
automatically. Before upgrading, run a v0.39 binary once to migrate it; otherwise,
move it to `~/.shine/shine.env.toml`, or merge its values there if that file already
exists. A normal config-loading command stops with recovery instructions while the
old file remains.

An active directory linked with `shine preset overlay link <path>` may also contain a
flat `<path>/shine.env.toml`. Its values override global env values and are
available from any working directory; project-local `shine.env.toml` values
still take priority. The file is re-read on every run, requires no project
`shine.config.toml`, and stops applying after `shine preset overlay unlink`. Overlays
also compose with a full external presets source: matching overlay paths win,
while other files continue to come from the external source.

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "Local proxy host" }
```

Like config `[env]` entries, every flat override value may be either a string or
an inline `{ value, description }` table. A detailed override replaces both the
value and its description; a string override replaces only the value and keeps
any description inherited from a lower-priority config or preset catalog.
Invalid value types are reported as errors rather than ignored.

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
    │   │   │   ├── Github Light Default
    │   │   │   └── Shine Light
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
