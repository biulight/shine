---
title: Initialize and manage a system
sidebar_position: 3
---

# Initialize and manage a system

Use system presets to install selected development tools on macOS, Ubuntu, Arch Linux, or Windows
and enable their shell integration. Shine can also manage system settings such as private DNS separately.
Ongoing software updates remain the responsibility of the package manager or upstream tool.

## Apply shell startup configuration {#refresh-initialization}

Installing Shine alone does not enable the system preset's startup configuration. For first use
or after editing that configuration, preview and apply it with the following commands. This
installs no software and preserves any enabled software integrations:

```bash
shine sys init --dry-run
shine sys init
```

Review the target shell and changes, then open a new shell after applying. Each new shell loads
the configuration; rerun `shine sys init` whenever you change the preset.

On Linux, switching between Bash and Zsh moves Shine's Sys configuration loading to the current
shell. For external presets, review the code using the
[authorization instructions](./custom-presets.md#declare-permissions) before applying.

## Inspect before installing {#inspect-before-applying}

List the items available for your system and inspect any prerequisites:

```bash
shine sys list
shine sys info neovim
```

The default list hides installation items whose declared shell integrations do not match Shine's
current target shell. Use `shine sys list --all` to see the complete catalog and integration ranges;
`shine sys info <ITEM>` also explains whether an item matches. Available items and named selections
(profiles) are listed in [Built-in presets](../built-in-presets/index.md#system-presets).

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
| Use a named selection | `shine sys bootstrap --preset <PROFILE>` |

Interactive choices and profile selections include only items matching the current shell, plus
items without declared shell integrations. Explicit IDs, including `--item`, bypass this filter;
installation can proceed even when no integration is declared for the current shell. Preview with
`--dry-run` to check the resulting selection.

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

For the built-in Arch selection and required full system update, follow
[Arch Linux preparation](../built-in-presets/system-bootstrap.md#arch-linux-preparation).

## Open a new terminal and check the result

Open a new shell after installation so that new commands and integrations become available.
To view saved results, run:

```bash
shine sys status
```

This shows the latest saved result for each item, possibly from different runs. It does not check
whether software is still installed or whether the current system configuration matches.

Software-specific steps, including Atuin history import, are in
[Ubuntu installation notes](../built-in-presets/system-bootstrap.md#ubuntu-installation-notes).

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
installed. Completing an incomplete package set may update its existing components; see the
[Ubuntu installation notes](../built-in-presets/system-bootstrap.md#ubuntu-installation-notes). `shine update` and `shine upgrade` manage Shine configuration,
not third-party software versions.

## Manage system settings {#managed-system-items}

Use `apply` and `uninstall` for managed settings such as `split-dns`. Before using that item,
complete the [private DNS setup](../built-in-presets/split-dns.md).

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
- **Shell configuration conflicts:** if the result says `needs action`, inspect the reported
  files. Shine preserves the conflicting file, but other files may already have changed. Use
  `shine sys init --force-profile` only if you want to back up and replace the conflicting
  profile instead of preserving and merging your edits.
- **Initialization refresh interrupted:** preview and rerun `shine sys init`; `sys recover`
  cannot roll back this refresh.

## Supported shells {#platform-notes}

Shell integration supports zsh on macOS, bash and zsh on Ubuntu and Arch Linux, and PowerShell on Windows.
For automatic terminal appearance settings on Ubuntu, Arch Linux, and macOS, see
[terminal theme synchronization](./terminal-theme-sync.md).

## Retire an older local HTTP service {#retire-legacy-http-service}

If you used Shine's old HTTP service, follow the
[cleanup steps in Upgrades and migration](./upgrading.md#retire-an-older-local-http-service).
