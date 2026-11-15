---
title: System setup selections
sidebar_position: 8
---

# System setup selections

These presets install selected development tools and configure their shell integration. Detected
software is skipped. Choose items or a group from the [profile contents](./index.md#system-presets)
to suit your machine.

For preview, approval, and shell integration controls, see
[Initialize and manage a system](../guides/system-init.md).

## Arch Linux preparation {#arch-linux-preparation}

Only official x86_64 Arch Linux is supported. The only preset is `minimal` (the default),
which includes Neovim, fzf, bat, eza, and zoxide.

Before installation, review Arch's update instructions and complete a full system update
(omit `sudo` as root). Once the update succeeds, preview and install:

```bash
sudo pacman -Syu

shine sys bootstrap --preset minimal --dry-run
shine sys bootstrap --preset minimal
```

Shine does not upgrade the system automatically. Arch does not support partial upgrades;
do not run `pacman -Sy` alone before installing packages.

If a download fails with HTTP 404, confirm that the system update completed successfully;
if the problem persists, check the configured mirrors. For other errors, follow pacman's output.
See [Arch system maintenance](https://wiki.archlinux.org/title/System_maintenance).

## Ubuntu installation notes {#ubuntu-installation-notes}

- **Atuin:** open a new shell after installation and run `atuin import auto` to import existing
  history. Run `atuin register` or `atuin login` if you want history synchronization.
- **Yazi:** installs from the official stable APT repository with preview dependencies. It requires
  administrator access, but not Rust or Cargo. Existing installations are skipped without changing
  their installation method.
- **pnpm:** does not require Node.js, npm, or Corepack beforehand. It installs under
  `~/.local/share/pnpm`; Shine configures the command path. Open a new shell to use it.
- **Minimal server setup:** `minimal` includes only the tools in its [profile row](./index.md#system-presets), without history sync,
  a prompt, the Node.js toolchain, or Homebrew. Review the actual steps with
  `shine sys bootstrap --preset minimal --dry-run` before installing.

## Rust commands missing on an older Ubuntu setup

Upgrade Shine, run `shine sys bootstrap rust`, and open a new shell. This repairs integration without
reinstalling detected Rust. In Bash/Zsh, you can also run `. "$HOME/.cargo/env"` for the current session
if that file exists.
