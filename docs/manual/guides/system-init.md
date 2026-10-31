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

Interactive lifecycle Plan confirmation can authorize the listed external code for that operation only,
without saving trust. Automation and `--yes` require an existing grant. Shell live always requires
Development trust; explicit `--run-generators` inspection also requires a grant.

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

For item IDs beginning with `-`, put options before `--`, then the item name, for example
`shine sys apply --dry-run -- -custom`.

## Select interactively or apply a profile

```bash
shine sys bootstrap
shine sys bootstrap mise
shine sys bootstrap rust mise
shine sys bootstrap --item rust --item mise --yes
shine sys bootstrap --preset recommended
shine sys bootstrap --preset minimal
shine sys bootstrap --proxy --dry-run
```

- In an interactive terminal, `shine sys bootstrap` opens a multi-select interface.
- Positional item IDs bootstrap only those items, preserving their order and ignoring duplicates.
- Repeated `--item` is the explicit equivalent for scripts and setup orchestrators.
- `--preset` applies the named profile directly.
- Positional items, repeated `--item`, and `--preset` cannot be combined. Managed resources use `sys apply`, not
  `sys bootstrap`.
- In a non-interactive environment without an explicit profile, Shine uses the configured default.

After selection, a mutating run displays the planned changes and required permissions. Interactive
approval defaults to No. Automation must pass `--yes`; this skips only the prompt, while the Plan is
still shown and validated. `--dry-run` is an earlier preview and cannot be combined with `--yes`.

Review the Plan's administrator access, network access, and recovery limitations before approving.
Use `--verbose` for detailed paths and steps. A permission declaration is not a safety guarantee:
custom scripts and executable profile content run with your permissions and are not sandboxed.
External code requires review and authorization.

Installers may prompt for terminal input. Follow the current item's instructions and wait for it to
finish before starting another operation.

On Ubuntu, the built-in Atuin installer runs non-interactively. Shine detects its default
`~/.atuin/bin` installation before the current shell has loaded the new PATH, then enables its
shell integration. After opening a new shell, run
`atuin import auto` to import existing history, and `atuin register` or `atuin login` if you want
history synchronization. Bootstrap runs items sequentially, allows up to 30 minutes per installer,
and stops at the first failed or timed-out item. Large installer logs are truncated to their final
output with a notice; log volume alone does not stop installation.

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

This view shows the latest recorded result for each item, including bootstrap and managed system
operations such as `shine sys apply`. Entries can come from different runs. It does not perform
live checks of software versions or system configuration.

## Upgrade bootstrap software with its owner

`shine sys bootstrap` is an ensure-present initializer, not a software update manager. Shine does
not check or upgrade bootstrap software. Use the package manager or upstream tool that owns each
item—such as Homebrew, apt, winget, mise, or rustup—to check for and install updates. Rerunning
`shine sys bootstrap` only verifies that the selected software is present and does not upgrade it.

The built-in mise step follows the same boundary: it can install mise and add activation to managed
shell profile content, but it does not create or update mise configuration and does not manage
runtime versions. Use mise for `mise.toml`, tool installation, and version upgrades.

## Manage shell integration state

Rust bootstrap makes the default Cargo binary directory (`~/.cargo/bin`) available in Ubuntu
Bash/Zsh, macOS Zsh, and Windows PowerShell profiles. Open a new shell after bootstrap to use
`rustup`, `cargo`, and `rustc`; bootstrap cannot change the calling shell's PATH.

If an older Ubuntu setup reports Rust as installed but the commands are missing, upgrade Shine,
then run `shine sys bootstrap rust` and open a new shell. This repairs the shell integration without
reinstalling an already detected Rust installation. For the current Bash/Zsh session, you can run
`. "$HOME/.cargo/env"` if that file exists.

A successful bootstrap enables only the selected items' declared shell integrations. It does not
disable integrations enabled by an earlier run, and a named selection profile is not a desired-state
replacement for software or shell configuration.

```bash
shine sys profile disable mise --dry-run
shine sys profile disable mise
shine sys profile enable mise
```

Declared PATH integrations add a directory only when it is absent; reloading the generated profile
does not add duplicate entries, including paths containing spaces or shell pattern characters.

These commands modify only Shine-owned generated profile content. Disabling does not uninstall the
software. Enabling first verifies the item's declared detection and asks you to bootstrap it when it
is missing. A mutating run displays and freshly validates a snapshot-bound security Plan;
automation must pass `--yes`, while `--dry-run` remains a separate preview. `shine upgrade` does not
change or re-render profile enablement implicitly; use these explicit profile commands when that
state should change.

Profile commands distinguish completed changes, an already enabled/disabled integration, and
`needs action`. If profile merging needs manual attention, the current profile is preserved and
the output points to the files to review. This warning alone does not cause a nonzero exit status;
check the result before assuming the requested integration is active.

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
shine sys recover              # Review an interrupted managed operation
shine sys recover --yes        # Non-interactive approval
```

For managed system items, `shine update` compares the desired configuration with the installation
record; `shine upgrade` also checks the actual system resource. On Windows, a split-DNS NRPT rule
that matches both the recorded and desired configuration needs no update or administrator access.
Use `shine upgrade shell/<CATEGORY>` to limit an upgrade to Shell presets.

Non-dry-run managed operations display a snapshot-bound Plan and default to No. `--yes` skips only
the prompt, not Plan rendering, permission blockers, or fresh validation. Administrator access, if
required, is requested separately after Plan approval.

If another Sys operation is running, wait for it to finish. If relevant state changes while waiting,
review the new Plan before retrying.

If a managed operation is interrupted, later Sys changes stop and ask you to run
`shine sys recover`. Recovery completes or rolls back only state that has not changed since the
interruption. It never overwrites modified resources or unrelated shell-profile content.

Bootstrap scripts and package-manager operations are not undone by `sys recover`; follow the
reported item and its package manager when they fail partway through.

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
