---
title: Initialize and manage a system
sidebar_position: 3
---

# Initialize and manage a system

System presets provide selectable development-environment initialization steps for macOS, Ubuntu,
and Windows, plus a small set of reversible managed system resources. They are not a general machine
configuration or package-version manager. Use `shine sys list` from the installed version as the
authoritative list of available items.

See [built-in presets](../reference/built-in-presets.md#system-presets) for each platform's profiles
and for `split-dns` environment variables and safe preview steps.

## Inspect before applying

```bash
shine sys list
shine sys list --all
shine sys info split-dns
shine sys bootstrap --dry-run
```

`--dry-run` shows selection results, script invocations, and managed profile updates without changing
the system. Some items require administrator access or additional environment variables;
`shine sys info <ITEM>` lists those requirements.

## Select interactively or apply a profile

```bash
shine sys bootstrap
shine sys bootstrap --preset recommended
shine sys bootstrap --preset minimal
shine sys bootstrap --proxy --dry-run
```

- In an interactive terminal, `shine sys bootstrap` opens a multi-select interface.
- `--preset` applies the named profile directly.
- In a non-interactive environment without an explicit profile, Shine uses the configured default.

Ubuntu includes a `minimal` profile for production servers. It installs only Neovim, fzf, bat, eza,
and zoxide, without shell-history synchronization, a prompt, the Node.js toolchain, or Homebrew.
Always review the current steps with `shine sys bootstrap --preset minimal --dry-run` first.

Add `--proxy` when downloads require an HTTP proxy. Shine derives uppercase and lowercase proxy
variables from `PROXY_HOST`, `HTTP_PROXY_PORT`, and `PROXY_NO_PROXY` in `[env]`; the default address
is `http://127.0.0.1:6152`. Combine it with `--dry-run` to inspect the effective values.

Windows `winget` does not read these environment variables, so Shine also passes
`winget install --proxy`. If that option is disabled, first run in an administrator PowerShell:

```powershell
winget settings --enable ProxyCommandLineOptions
```

After initialization, inspect the recorded state:

```bash
shine sys status
```

This view is a run record: `installed`, `already installed`, or `completed` describes what the last
bootstrap invocation observed. It does not probe whether third-party software is still present or
current. Use the read-only update command below for the supported live checks.

## Check bootstrap software updates

Read-only update checks are available for software recorded during initialization:

```bash
shine sys update
shine sys update neovim --verbose
shine sys update --proxy
```

This command checks only software recorded by `shine sys bootstrap`. It does not install or upgrade
software and does not modify the system manifest or shell profile. By default it shows only packages
that a package manager confirms have updates, together with copyable upstream upgrade commands.
`--verbose` also shows current packages and items that require a manual check.

Built-in presets currently check through Homebrew, apt, and winget. Direct installers and
user-maintained Git configuration are marked for manual review rather than assigned a guessed
version. `--proxy` uses the same configuration as `sys bootstrap --proxy`; on Windows, it explicitly
passes winget's `--proxy` option.

On Ubuntu, Shine does not record the installation source of software it finds already present.
Manual-check results therefore avoid guessing an updater and give source-specific guidance only
when it is safe to do so: for example, standalone `mise.run` installs use `mise self-update`, while
package-managed installs use their original package manager. Rerunning `shine sys bootstrap` only
verifies that existing software is present and does not upgrade it.

The built-in mise step follows the same boundary: it can install mise and add activation to managed
shell profile content, but it does not create or update mise configuration and does not manage
runtime versions. Use mise for `mise.toml`, tool installation, and version upgrades.

`shine update` and `shine upgrade` still manage only Shine configuration and managed system
resources. They never upgrade this third-party software. You decide whether to execute commands
printed by `shine sys update`.

Top-level `shine list` includes managed system configuration recorded for the current operating
system. Use `shine sys status` and `shine sys info <ITEM>` for details. `update --verbose` and
`upgrade --verbose` also show skipped, current, and attention-required managed resources.

## Managed system items

Some system configuration is managed declaratively and can be reapplied or safely removed:

```bash
shine sys apply --dry-run
shine sys apply split-dns
shine sys uninstall split-dns --dry-run
shine sys uninstall split-dns
```

For routing private domains across remote LANs to ZeroTier DNS, see the Chinese Biulight guide
[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns).

On Ubuntu, `split-dns` expects applications to query the `127.0.0.53` systemd-resolved stub. Shine
warns or refuses to write ineffective configuration when the stub is disabled. Re-enable
`DNSStubListener`, or confirm that local resolution really passes through systemd-resolved, before
applying the item.

System profiles merge with and preserve user content where possible. Use
`shine sys bootstrap --force-profile` only when you explicitly want to back up and replace a
conflicting profile.

## Platform notes

- Managed shell profiles target zsh on macOS.
- Ubuntu supports bash and zsh.
- Windows system presets and profile integration use PowerShell.
- Ubuntu and macOS can detect the terminal background and set `SHINE_TERMINAL_THEME`; set
  `SHINE_SYNC_TERMINAL_THEME=0` to disable it.
