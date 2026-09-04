---
title: Built-in presets
sidebar_position: 3
---

# Built-in presets

Shine compiles these presets into the CLI. They are usable defaults and copyable starting points,
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

This page reflects the built-in `presets/` directory in Shine 2.0.1. For another version, use
`shine list --available` and `--help` as the authority.

The platform column below is generated from the same built-in App destination and Shell file
selectors used at runtime. A conformance test keeps this block synchronized with preset metadata in
both manual locales.

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

Installing a shell category creates entries under `~/.shine/bin/` and wrappers in supported Bash,
Zsh, or PowerShell profiles. Commands described as acting on the current session are sourced by the
wrapper; do not bypass it and execute the script directly.

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
| `utils` | `shine-theme-sync` | Export `SHINE_TERMINAL_THEME` and `BAT_THEME` for the terminal appearance. |

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

Most categories install configuration files. Surge and Clash Verge Rev intentionally go further:
their provider-specific artifact scripts assemble the application-specific include/editor files
that users would otherwise have to wire by hand. The generic app lifecycle still owns the declared
files; the artifact remains an explicit, category-specific capability.

<div className="built-in-presets-app-table" aria-hidden="true" />

| Category | Platform and destination | Managed content and notes |
| --- | --- | --- |
| `archey4` | Unix; `~/.config/archey4/` | Archey4 system-information configuration. |
| `clash-verge` | macOS, Linux, Windows; merge source under `~/.shine/clash-verge/`, local rule references under CVR's user data directory | Clash Verge Rev subscription enhancement example. Option 1 uses managed HomeDir rule files; HTTP/HTTPS modes ignore them. Apply and teardown require Bun; see the [complete workflow](../guides/app-presets.md#clash-verge-rev). |
| `docker-desktop` | Windows; `~/AppData/Roaming/Docker/settings-store.json` | JSON-merge manages only `proxy` and `containersProxy`; restart Docker Desktop afterward. |
| `docker-engine` | Unix: `/etc/docker/daemon.json`; Windows: `~/.docker/daemon.json` | Template plus JSONC conversion. Unix needs administrator access; restart Docker Engine afterward. |
| `fastfetch` | `~/.config/fastfetch/` | Fastfetch system-information configuration. |
| `ghostty` | Unix; `~/.config/ghostty/` | Main configuration and built-in light/dark themes; backgrounds use `GHOSTTY_BG_LIGHT` and `GHOSTTY_BG_DARK`. |
| `git` | `~/.gitconfig` | Common aliases and defaults; an unmanaged existing file is backed up by normal app rules. |
| `JetBrains` | `~/.ideavimrc` | IdeaVim configuration; enable the IdeaVim plugin in the IDE. |
| `starship` | `~/.config/starship.toml` | Starship prompt configuration; install and enable Starship separately. |
| `surge` | macOS; `~/Library/Application Support/Surge/Profiles/` | Local proxies, groups, rules, and an optional generated URI subscription. See the [generated-file and artifact workflow](../guides/app-presets.md#generated-files-and-surge-uri-subscriptions). |
| `vim` | `~/.vim/` | Basic Vim configuration and a machine-local override. |

Docker Desktop JSON merge preserves every other setting. Other application presets manage only
their declared files. Installing a preset does not download, install, or start Ghostty, Docker,
Surge, Starship, or another application.

## System presets

`shine sys bootstrap` runs standardized package providers or isolated compatibility scripts for
selected development-environment items. Interactive mode selects items, positional IDs run only
those items, and non-interactive mode uses the platform default profile.
The recorded result means that the bootstrap step ran; it is not a live assertion that a package is
still installed or current. Rerunning bootstrap checks presence but does not upgrade third-party
software. Run `--dry-run` first because package managers, downloads, and profile merging may require
privileges or change the machine.

Shell integration is item-owned: a successful targeted bootstrap enables that item's generated
content without pulling in integrations for unselected software. Earlier enabled integrations stay
active until explicitly disabled with `shine sys profile disable <ITEM>`.

The `mise` item installs mise and activates it through Shine-managed shell profile content. Shine
does not write mise configuration or manage runtime versions; use mise itself for those operations.
Shine does not check or upgrade third-party bootstrap software; use Homebrew, apt, winget, mise,
rustup, or the upstream tool that owns the software.

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

It does not install ZeroTier or CoreDNS or create DNS zones. Set up the private network and DNS first.
See the Chinese guide
[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns).

See [Initialize and manage a system](../guides/system-init.md) for profiles, managed items, and proxy
usage.
