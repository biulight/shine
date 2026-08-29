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

`--dry-run` shows selection results, provider/script invocations, and each persistent item-owned
shell integration without changing the system. Some items require administrator access or additional environment variables;
`shine sys info <ITEM>` lists those requirements.

When a missing item is about to run, Shine prints its `sys/<ITEM>` identifier and label first, so
any authorization or password prompt that follows is attributable to the active software.

## Select interactively or apply a profile

```bash
shine sys bootstrap
shine sys bootstrap mise
shine sys bootstrap rust mise
shine sys bootstrap --preset recommended
shine sys bootstrap --preset minimal
shine sys bootstrap --proxy --dry-run
```

- In an interactive terminal, `shine sys bootstrap` opens a multi-select interface.
- Positional item IDs bootstrap only those items, preserving their order and ignoring duplicates.
- `--preset` applies the named profile directly.
- Positional items and `--preset` cannot be combined. Managed resources use `sys apply`, not
  `sys bootstrap`.
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
current.

## Upgrade bootstrap software with its owner

`shine sys bootstrap` is an ensure-present initializer, not a software update manager. Shine does
not check or upgrade bootstrap software. Use the package manager or upstream tool that owns each
item—such as Homebrew, apt, winget, mise, or rustup—to check for and install updates. Rerunning
`shine sys bootstrap` only verifies that the selected software is present and does not upgrade it.

The built-in mise step follows the same boundary: it can install mise and add activation to managed
shell profile content, but it does not create or update mise configuration and does not manage
runtime versions. Use mise for `mise.toml`, tool installation, and version upgrades.

## Manage shell integration state

A successful bootstrap enables only the selected items' declared shell integrations. It does not
disable integrations enabled by an earlier run, and a named selection profile is not a desired-state
replacement for software or shell configuration.

```bash
shine sys profile disable mise --dry-run
shine sys profile disable mise
shine sys profile enable mise
```

These commands modify only Shine-owned generated profile content. Disabling does not uninstall the
software. Enabling first verifies the item's declared detection and asks you to bootstrap it when it
is missing. `shine upgrade` does not change or re-render profile enablement implicitly; use these
explicit profile commands when that state should change.

`shine update` and `shine upgrade` still manage only Shine configuration and managed system
resources. They never upgrade this third-party software.

Top-level `shine list` includes managed system configuration recorded for the current operating
system. Use `shine sys status` and `shine sys info <ITEM>` for details. `update --verbose` and
`upgrade --verbose` also show skipped, current, and attention-required managed resources.

## Managed system items

Some system configuration is managed declaratively and can be reapplied or safely removed:

```bash
shine sys apply --dry-run
shine sys apply split-dns
shine sys apply split-dns --yes # Non-interactive approval
shine sys uninstall split-dns --dry-run
shine sys uninstall split-dns
```

Non-dry-run managed operations display a snapshot-bound Plan and default to No. `--yes` skips only
the prompt, not Plan rendering, permission blockers, or fresh validation. Administrator access, if
required, is requested separately after Plan approval.

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
