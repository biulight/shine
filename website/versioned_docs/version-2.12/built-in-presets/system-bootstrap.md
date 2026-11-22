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

## Installation methods by platform {#installation-methods}

The built-in `shine sys bootstrap` presets use the installation method specified for each item.

| Platform | Installation method and prerequisites |
| --- | --- |
| macOS | Most tools use Homebrew, including Homebrew Cask for some apps. Rust uses a separate installer. Built-in profiles list Homebrew first. When selecting an individual tool that uses Homebrew, make sure `brew` is available. |
| Windows | Software installation items use WinGet. Make sure `winget` is available before installing. |
| Ubuntu | Items use APT or a separate installer. For example, Neovim uses APT and Rust uses the rustup installer. |
| Arch Linux | The `minimal` profile uses pacman. [Complete a full system update](#arch-linux-preparation) before installing. |

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
- **Docker Engine:** included in `recommended` and `all`. Follows the
  [official Ubuntu APT installation](https://docs.docker.com/engine/install/ubuntu/#install-using-the-apt-repository):
  removes conflicting distribution packages, configures Docker's signed stable repository, and installs
  Engine, CLI, containerd, Buildx and Compose. Requires administrator access. Removing conflicts such as
  `docker.io`, `podman-docker`, `containerd` or `runc` can affect existing workloads and dependent packages;
  review the installation plan before continuing. Container data is not deleted.
  Installation is skipped only when all five official packages are fully installed and no conflicting
  package remains; a `dockerd` command alone is insufficient. Missing components are installed through APT, which may also update other packages in this set.
  After installation, Shine starts Docker if needed and runs `hello-world` to verify it; network access
  to Docker Hub is required. Use `sudo docker`; Shine does not add your user to the `docker` group.
  Review Docker's firewall notes before publishing container ports.
- **pnpm:** does not require Node.js, npm, or Corepack beforehand. It installs under
  `~/.local/share/pnpm`; Shine configures the command path. Open a new shell to use it.
- **Minimal server setup:** `minimal` includes only the tools in its [profile row](./index.md#system-presets), without history sync,
  a prompt, the Node.js toolchain, or Homebrew. Review the actual steps with
  `shine sys bootstrap --preset minimal --dry-run` before installing.

## Rust commands missing on an older Ubuntu setup

Upgrade Shine, run `shine sys bootstrap rust`, and open a new shell. This repairs integration without
reinstalling detected Rust. In Bash/Zsh, you can also run `. "$HOME/.cargo/env"` for the current session
if that file exists.
