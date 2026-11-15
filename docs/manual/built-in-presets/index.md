---
title: Overview and selection
sidebar_position: 1
slug: /built-in-presets
---

# Overview and selection

Shine includes common scripts, application configurations, and system setup selections prepared by
the author. They reflect the author's everyday preferences; choose or customize the ones you need.

## Shell presets

<div className="built-in-presets-shell-table" aria-hidden="true" />

| Category | Command | Purpose and key requirements |
| --- | --- | --- |
| `agent` | [`ccenv`](./agent.md) | Launch Claude Code and select provider/model settings. Supports credentials encrypted in Shine to avoid long-term plaintext keys in startup files. Requires Bun and Claude Code. |
| `image-tools` | [`img-compress`](./image-tools.md) | Batch-compress images while keeping originals; output is not guaranteed to be smaller. |
| `image-tools` | [`img-resize`](./image-tools.md) | Batch-resize images within chosen dimensions without enlarging or overwriting originals. |
| `image-tools` | [`img-convert`](./image-tools.md) | Batch-convert image formats while keeping originals. |
| `proxy` | [`setproxy`](./proxy.md) | Configure terminal and package-manager proxies. Requires an existing proxy server; also changes persistent Yarn settings. |
| `proxy` | [`usetproxy`](./proxy.md) | Clear proxy-related session variables and Yarn proxy entries; does not restore previous values. |
| `utils` | [`copyfile`](./utils.md#copy-a-file-to-the-clipboard) | Copy file contents to the local clipboard. Unix only; terminal or multiplexer must allow OSC 52. |
| `utils` | [`shine-env-export`](./utils.md#export-a-value) | Load a Shine value into the current shell, decrypting it when needed. Exported values remain in that session. |
| `utils` | [`shine-theme-sync`](./utils.md#terminal-theme) | Apply terminal-theme variables to the current session. |

The image tools support JPEG, PNG, and WebP and require Bun 1.3.14 or newer.

## Application presets

These presets provide application configuration; install the applications separately.

<div className="built-in-presets-app-table" aria-hidden="true" />

| Category | Configuration and selection notes |
| --- | --- |
| `archey4` | Host, CPU, memory, disk, and network summary layout. |
| [`clash-verge`](./clash-verge.md) | Local proxies, groups, and rules alongside a subscription. Requires Bun; customize the inactive example and connect it to the subscription editors. |
| `docker-desktop` | Docker Desktop and container proxy settings. Windows only; replaces the complete `proxy` and `containersProxy` sections, preserving settings outside them. Restart Docker Desktop. |
| `docker-engine` | Daemon proxy settings from Shine variables. Replaces the complete `daemon.json`; review existing settings first. Unix requires administrator access; restart the daemon. |
| `fastfetch` | Hardware, OS, network, and appearance summary layout. |
| `ghostty` | Light/dark appearance and split-navigation bindings, including Ctrl + H/J/K/L to switch panes. |
| `git` | Status, branch, and graph-log aliases with Git defaults. Enables rebasing pulls and pushing the current branch. |
| `JetBrains` | IdeaVim editing and pane-navigation mappings. Enable the IdeaVim plugin. |
| `starship` | Prompt settings for time, Git information, and command outcome. Enable Starship in your shell. |
| [`surge`](./surge.md) | Local proxies, policy groups, and rules; optional URI subscriptions. macOS; requires Bun and running Surge with CLI module support. Customize the local configuration before applying. |
| `vim` | Editing, search, indentation, and key mappings, with a machine-specific override file. Review the theme and plugin requirements. |

## System presets

The following profiles select development tools to install. They do not upgrade existing software.
The `mise` item installs and activates mise; tool versions are configured through mise itself.

<div className="built-in-presets-system-table" aria-hidden="true" />

| Platform | Profile | Included items |
| --- | --- | --- |
| macOS | `required` | Homebrew, Yazi, Starship. |
| macOS | `recommended` (default) | `required` plus Rust, Neovim, AstroNvim, ZeroTier, Zsh plugins, zoxide, Atuin, fzf, bat, eza. |
| macOS | `all` | `recommended` plus nvm, Bun, pnpm, mise, Fastfetch. |
| Ubuntu | `recommended` (default) | Rust, Neovim, AstroNvim, Atuin, Yazi, Starship, zoxide, zsh-vi-mode, fzf, bat, eza. |
| Ubuntu | `all` | `recommended` plus ZeroTier, Bun, pnpm, mise, Homebrew. |
| Ubuntu | `minimal` | Neovim, fzf, bat, eza, zoxide; for servers without history synchronization, a prompt, or JavaScript tooling. |
| Arch Linux | `minimal` (default) | Neovim, fzf, bat, eza, zoxide via pacman; Bash/Zsh integration. Official x86_64 Arch Linux only. |
| Windows | `required` | Rust, Yazi, Starship. |
| Windows | `recommended` (default) | `required` plus Neovim, zoxide, Atuin, fzf, bat, eza, ZeroTier. |
| Windows | `all` | `recommended` plus Bun, pnpm, mise. |

[System setup notes](./system-bootstrap.md) cover platform prerequisites and follow-up steps;
Arch requires a [full system update before installation](./system-bootstrap.md#arch-linux-preparation).
[Private split DNS](./split-dns.md) is a separate optional item for macOS, Ubuntu, and Windows,
outside these profiles. It requires administrator access and a reachable DNS server.

## Platform support {#platform-support}

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

## Use and customize {#use-and-customize}

For installation, updates, removal, permissions, and recovery, see the
[Shell](../guides/shell-presets.md), [App](../guides/app-presets.md), or
[system](../guides/system-init.md) guide. Preset-specific setup is linked from the tables above.

To change bundled defaults, use an [overlay](../guides/custom-presets.md#override-selected-files-with-an-overlay).
Variable settings are covered in the [environment guide](../guides/environment.md).
