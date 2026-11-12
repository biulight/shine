---
title: Built-in presets
sidebar_position: 3
---

# Built-in presets

Shine includes these presets as usable defaults and copyable starting points,
not an exhaustive catalog of every tool Shine can support. Shell and app presets manage declared
commands and configuration; system presets initialize selected development tools or manage a small
set of reversible system resources. App presets do not automatically install the corresponding
desktop application. Inspect and preview before applying to confirm destinations, backups, and
permissions:

```bash
shine shell list
shine app list
shine app info ghostty
shine app install ghostty --dry-run
shine sys list --all
shine sys info split-dns
```

This page describes the presets in the current development version (Next). Use
`shine list --available` and `--help` to check what your installed version provides.

The table below shows platform availability.

<!-- BEGIN GENERATED PRESET PLATFORM CAPABILITIES -->
| Preset capability | macOS | Linux | Windows |
| --- | --- | --- | --- |
| `app/JetBrains` | ✓ | ✓ | ✓ |
| `app/archey4` | ✓ | ✓ | — |
| `app/clash-verge` | ✓ | ✓ | ✓ |
| `app/docker-desktop` | — | — | ✓ |
| `app/docker-engine` | ✓ | ✓ | ✓ |
| `app/fastfetch` | ✓ | ✓ | ✓ |
| `app/ghostty` | ✓ | ✓ | — |
| `app/git` | ✓ | ✓ | ✓ |
| `app/starship` | ✓ | ✓ | ✓ |
| `app/surge` | ✓ | — | — |
| `app/vim` | ✓ | ✓ | ✓ |
| `shell/agent/ccenv` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-compress` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-convert` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-resize` | ✓ | ✓ | ✓ |
| `shell/proxy/setproxy` | ✓ | ✓ | ✓ |
| `shell/proxy/usetproxy` | ✓ | ✓ | ✓ |
| `shell/utils/copyfile` | ✓ | ✓ | — |
| `shell/utils/shine-env-export` | ✓ | ✓ | ✓ |
| `shell/utils/shine-theme-sync` | ✓ | ✓ | ✓ |
<!-- END GENERATED PRESET PLATFORM CAPABILITIES -->

## Shell presets

After installing a Shell category, open a new Bash, Zsh, or PowerShell session and use the command
names below. Use these names for commands that change the current session too; do not run the
script files from the preset directory directly.

<div className="built-in-presets-shell-table" aria-hidden="true" />

| Category | Command | Purpose and prerequisites |
| --- | --- | --- |
| `agent` | `ccenv` | Use Bun on macOS, Linux, or Windows to select Codex (default), DeepSeek, or Qwen and launch Claude Code. Provider values enter only that child. Requires `bun`, `shine`, and Claude Code. |
| `image-tools` | `img-compress` | Batch-compress direct JPEG, PNG, and WebP file or directory inputs into derived output files. Uses `IMAGE_QUALITY`; requires Bun 1.3.14 or newer. |
| `image-tools` | `img-resize` | Batch-resize images to fit `IMAGE_MAX_WIDTH` × `IMAGE_MAX_HEIGHT` without enlargement. Requires Bun 1.3.14 or newer. |
| `image-tools` | `img-convert` | Batch-convert images to JPEG, PNG, or WebP with an explicit `--format`. Requires Bun 1.3.14 or newer. |
| `proxy` | `setproxy` | Set HTTP/HTTPS/SOCKS5, npm, and pnpm proxies for the current session from `HTTP_PROXY_PORT`, `SOCKS5_PROXY_PORT`, `PROXY_HOST`, and `PROXY_NO_PROXY`; `auto` prefers SOCKS5. Also changes persistent Yarn proxy settings when Yarn exists. |
| `proxy` | `usetproxy` | Clear session proxy variables and Yarn settings written by `setproxy`. |
| `utils` | `copyfile` | Copy a file to the local clipboard through OSC 52. Unix only; the terminal or multiplexer must allow OSC 52. |
| `utils` | `shine-env-export` | Load a Shine value into the current session, using plaintext or decrypting `<KEY>_SECRET`. |
| `utils` | `shine-theme-sync` | Set `SHINE_TERMINAL_THEME` and `BAT_THEME` for the current session’s terminal appearance. |

The Codex provider in `ccenv` uses local CLIProxyAPI with `CLIPROXYAPI_AUTH_TOKEN`; DeepSeek and Qwen
use their matching `*_API_KEY`. Resolution order is `_SECRET`, legacy `_GPG_SECRET`, then plaintext.
Failure to decrypt a selected ciphertext stops without falling back. Bind CLIProxyAPI to loopback and
configure the same token on both sides.

Image commands accept multiple files or one-level directory scans, derive new files instead of
editing inputs, and accept `--output-dir` plus explicit command-specific overrides. See the
[complete image workflow](../guides/custom-presets.md#from-source-folders-to-installed-capabilities).

See [Manage shell presets](../guides/shell-presets.md) and
[Manage environment variables](../guides/environment.md).

## Application presets

Most presets install configuration files. Surge and Clash Verge Rev also provide explicit artifact
commands to connect those files to the active application configuration; follow their setup guides.

<div className="built-in-presets-app-table" aria-hidden="true" />

| Category | Platform and destination | Managed content and notes |
| --- | --- | --- |
| `archey4` | Unix; `~/.config/archey4/` | Archey4 system-information configuration. |
| `clash-verge` | macOS, Linux, Windows; merge source under `~/.shine/clash-verge/`, local rule references under CVR's user data directory | Clash Verge Rev subscription enhancement example. Option 1 uses managed HomeDir rule files; HTTP/HTTPS modes ignore them. Apply and teardown require Bun; see the [complete workflow](../guides/app-presets.md#clash-verge-rev). |
| `docker-desktop` | Windows; `~/AppData/Roaming/Docker/settings-store.json` | Changes only `proxy` and `containersProxy`; restart Docker Desktop afterward. |
| `docker-engine` | Unix: `/etc/docker/daemon.json`; Windows: `~/.docker/daemon.json` | Docker daemon configuration. Unix needs administrator access; restart Docker Engine afterward. |
| `fastfetch` | `~/.config/fastfetch/` | Fastfetch system-information configuration. |
| `ghostty` | Unix; `~/.config/ghostty/` | Main configuration and built-in light/dark themes; backgrounds use `GHOSTTY_BG_LIGHT` and `GHOSTTY_BG_DARK`. Ctrl + `\` creates a split to the right; Ctrl + `-` creates one below. Ctrl + H/J/K/L focuses the left/down/up/right split; Super + [ / ] is unbound (Cmd on macOS). When no split can be selected, the keys reach the terminal application. |
| `git` | `~/.gitconfig` | Common aliases and defaults; an unmanaged existing file is backed up by normal app rules. |
| `JetBrains` | `~/.ideavimrc` | IdeaVim configuration; enable the IdeaVim plugin in the IDE. In Normal mode, Ctrl + H/J/K/L focuses the left/down/up/right editor split, matching the Ghostty preset. |
| `starship` | `~/.config/starship.toml` | Starship prompt configuration; install and enable Starship separately. |
| `surge` | macOS; `~/Library/Application Support/Surge/Profiles/` | Local proxies, groups, rules, a General-settings module, and an optional generated URI subscription. Artifact apply/remove requires Bun and running Surge with CLI module support. See the [generated-file and artifact workflow](../guides/app-presets.md#generated-files-and-surge-uri-subscriptions). |
| `vim` | `~/.vim/` | Basic Vim configuration and a machine-local override. |

Docker Desktop JSON merge preserves every other setting. Other application presets manage only
their declared files. Installing a preset does not download, install, or start Ghostty, Docker,
Surge, Starship, or another application.

## System presets

Use `shine sys bootstrap` to install selected development tools, or choose a group from the table
with `--preset`. Preview the steps and permissions with `--dry-run` first. Follow
[Initialize and manage a system](../guides/system-init.md) for the complete workflow.

Installation enables the selected tools' shell integrations and keeps previously enabled ones.
Use `shine sys profile disable <ITEM>` to turn one off. Shine does not upgrade installed software;
use its package manager or upstream tool. The `mise` item only installs and activates mise;
use mise itself for tool configuration and version management.

<div className="built-in-presets-system-table" aria-hidden="true" />

| Platform | Profile | Included items |
| --- | --- | --- |
| macOS | `required` | Homebrew, Yazi, Starship. |
| macOS | `recommended` (default) | `required` plus Rust, Neovim, AstroNvim, ZeroTier, Zsh plugins, zoxide, Atuin, fzf, bat, eza. |
| macOS | `all` | `recommended` plus nvm, Bun, pnpm, mise, Fastfetch. |
| Ubuntu | `recommended` (default) | Rust, Neovim, AstroNvim, Atuin, Yazi, Starship, zoxide, zsh-vi-mode, fzf, bat, eza. |
| Ubuntu | `all` | `recommended` plus ZeroTier, Bun, pnpm, mise, Homebrew. |
| Ubuntu | `minimal` | Neovim, fzf, bat, eza, zoxide; for servers without history sync, prompt, or JavaScript tooling. |
| Windows | `required` | Rust, Yazi, Starship. |
| Windows | `recommended` (default) | `required` plus Neovim, zoxide, Atuin, fzf, bat, eza, ZeroTier. |
| Windows | `all` | `recommended` plus Bun, pnpm, mise. |

### Ubuntu installation notes {#ubuntu-installation-notes}

- **Atuin:** open a new shell after installation and run `atuin import auto` to import existing
  history. Run `atuin register` or `atuin login` if you want history synchronization.
- **Yazi:** installs from the official stable APT repository with preview dependencies. It requires
  administrator access, but not Rust or Cargo. Existing installations are skipped without changing
  their installation method.
- **pnpm:** does not require Node.js, npm, or Corepack beforehand. It installs under
  `~/.local/share/pnpm`; Shine configures the command path. Open a new shell to use it.
- **Minimal server setup:** `minimal` includes only the tools listed above, without history sync,
  a prompt, the Node.js toolchain, or Homebrew. Review the actual steps with
  `shine sys bootstrap --preset minimal --dry-run` before installing.

### Private split DNS

All three platforms include an independent managed `split-dns` item outside default profiles. It
routes a private suffix to a DNS server reachable over ZeroTier and requires administrator access and
these effective values:

```toml
PRIVATE_DNS_DOMAIN = "home.example.internal"
PRIVATE_DNS_SERVERS = "10.0.0.53"
```

Preview before applying or removing:

```bash
shine sys info split-dns
shine sys apply split-dns --dry-run
shine sys apply split-dns
shine sys uninstall split-dns --dry-run
```

On Ubuntu, `split-dns` requires local DNS queries to pass through the `127.0.0.53`
systemd-resolved stub. If Shine reports that the stub is disabled, re-enable `DNSStubListener`
or confirm that local resolution passes through systemd-resolved before applying the configuration.

It does not install ZeroTier or CoreDNS or create DNS zones. Set up the private network and DNS first.
See the Chinese guide
[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns).

See [Initialize and manage a system](../guides/system-init.md) for profiles, managed items, and proxy
usage.
