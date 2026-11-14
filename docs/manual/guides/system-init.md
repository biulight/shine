---
title: Initialize and manage a system
sidebar_position: 3
---

# Initialize and manage a system

Use system presets to install selected development tools on macOS, Ubuntu, Arch Linux, or Windows
and enable their shell integration. Shine can also manage system settings such as private DNS separately.
It does not upgrade third-party software.

## Inspect before installing {#inspect-before-applying}

List the items available for your system and inspect any prerequisites:

```bash
shine sys list
shine sys info neovim
```

Use `shine sys list --all` to browse other platforms. Available items and named selections
(profiles) are listed in [Built-in presets](../reference/built-in-presets.md#system-presets).

Preview the items you want to install:

```bash
shine sys bootstrap neovim fzf --dry-run
```

The preview shows the installation steps and shell changes without applying them. Check any
administrator access, downloads, or required environment variables before continuing.

## Choose and install tools {#select-interactively-or-apply-a-profile}

Choose one method:

| Goal | Command |
| --- | --- |
| Choose items interactively | `shine sys bootstrap` |
| Install only fzf | `shine sys bootstrap fzf` |
| Install Neovim, then fzf | `shine sys bootstrap neovim fzf` |
| Use the recommended selection (macOS, Ubuntu, Windows) | `shine sys bootstrap --preset recommended` |
| Use the minimal selection (Ubuntu, Arch Linux) | `shine sys bootstrap --preset minimal` |

Before changing the system, Shine shows a plan and asks for confirmation; the default is No.
Review the changes, permissions, and recovery limits. Add `--verbose` if you need more detail.
Custom scripts run with your permissions without sandbox isolation; approve external code only
after reviewing it.

Follow any installer prompts and wait for the operation to finish. Items run in order, and Shine
stops at the first failure or timeout; each installer has a 30-minute limit. Use Shine's final
result to confirm success, rather than a success message within an installer's log.

For scripts, see [system command options](../reference/commands.md#system-presets), including
`--yes` and the authorization required for external code.

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

## Open a new terminal and check the result

Open a new shell after installation so that new commands and integrations become available.
To view saved results, run:

```bash
shine sys status
```

This shows the latest saved result for each item, possibly from different runs. It does not check
whether software is still installed or whether the current system configuration matches.

Software-specific steps, including Atuin history import, are in
[Ubuntu installation notes](../reference/built-in-presets.md#ubuntu-installation-notes).

## Use a download proxy

Add `--proxy` to use Shine's HTTP proxy settings. Preview first, then install:

```bash
shine sys bootstrap fzf --proxy --dry-run
shine sys bootstrap fzf --proxy
```

Configure `PROXY_HOST`, `HTTP_PROXY_PORT`, and `PROXY_NO_PROXY` in Shine's
[environment settings](./environment.md). The default address is `http://127.0.0.1:6152`;
check the effective values in the preview.

On Windows, winget may first require this command in an administrator PowerShell:

```powershell
winget settings --enable ProxyCommandLineOptions
```

## Enable or disable shell integration {#manage-shell-integration-state}

Shell integration makes a tool available in your terminal or enables features such as its prompt
or history search. Installing selected items enables their integrations and keeps previously
enabled integrations active, even when you choose a different profile.

| Goal | Command |
| --- | --- |
| Preview disabling fzf integration | `shine sys profile disable fzf --dry-run` |
| Disable it | `shine sys profile disable fzf` |
| Enable it again | `shine sys profile enable fzf` |

These commands change only Shine-managed shell configuration. Disabling does not uninstall the
software; enabling requires it to be installed first. `shine upgrade` does not enable or disable
these integrations. Open a new shell to use the updated configuration.

If the result says `needs action`, inspect the files identified in the output. Shine preserves the
current file when a merge needs manual attention; a successful command exit alone does not mean
the requested integration is active.

## Upgrade installed software {#upgrade-bootstrap-software-with-its-owner}

Use the package manager or upstream tool that installed the software, such as Homebrew, apt, pacman,
winget, mise, or rustup. Rerunning `shine sys bootstrap` skips software it detects as already
installed; it does not upgrade it. `shine update` and `shine upgrade` manage Shine configuration,
not third-party software versions.

For mise, use mise itself to configure and install tools and manage their versions.

## Manage system settings {#managed-system-items}

Use `apply` and `uninstall` for managed settings such as `split-dns`. Before using that item,
complete the [private DNS setup](../reference/built-in-presets.md#private-split-dns).

| Goal | Command |
| --- | --- |
| Preview private DNS changes | `shine sys apply split-dns --dry-run` |
| Apply them | `shine sys apply split-dns` |
| Preview removal | `shine sys uninstall split-dns --dry-run` |
| Remove the configuration | `shine sys uninstall split-dns` |

Review the plan before confirming. If administrator access is needed, Shine requests it after
approval. Use `shine sys info split-dns` for details and `shine sys status` for saved results.

## Handle failures and conflicts

- **Installation failed or timed out:** check the named item's error and follow its installer or
  package manager's repair instructions. `shine sys recover` does not undo software installation.
- **A managed operation was interrupted:** when Shine requests recovery, run `shine sys recover`
  and review its plan. Recovery completes or rolls back the interrupted operation only when the
  relevant state has not changed. It preserves modified resources and unrelated shell content.
- **Another system operation is running:** wait for it to finish. If Shine reports that the state
  changed, review the new plan before retrying.
- **Shell configuration conflicts:** inspect the files identified by Shine. Use
  `shine sys bootstrap --force-profile` only if you want to back up and replace the conflicting
  profile instead of preserving and merging your edits.
- **Rust commands are missing on an older Ubuntu setup:** upgrade Shine, run
  `shine sys bootstrap rust`, and open a new shell. This repairs integration without reinstalling
  detected Rust. In Bash/Zsh, you can also run `. "$HOME/.cargo/env"` for the current session if
  that file exists.

## Supported shells {#platform-notes}

Shell integration supports zsh on macOS, bash and zsh on Ubuntu and Arch Linux, and PowerShell on Windows.
For automatic terminal appearance settings on Ubuntu, Arch Linux, and macOS, see
[terminal theme synchronization](./terminal-theme-sync.md).

## Retire an older local HTTP service {#retire-legacy-http-service}

If you used Shine's old HTTP service, follow the
[cleanup steps in Upgrades and migration](./upgrading.md#retire-an-older-local-http-service).
