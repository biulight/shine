---
title: Installation and upgrades
sidebar_position: 2
---

# Installation and upgrades

Shine supports macOS, Linux, and Windows. The official install scripts download the binary for the
current platform from GitHub Releases.

## macOS and Linux

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

The default destination is `~/.local/bin/shine`. The installer does not modify shell configuration.
Home and installation paths containing newline characters are not supported.
If it reports that the directory is missing from `PATH`, run the printed configuration command,
then the command for your current terminal.
If a configuration file cannot be updated, follow the error message and configure it manually.
If your shell differs from the one shown, or you use Fish or another shell, add the directory to
its configuration manually. Verify the installation:

```bash
shine --version
```

To choose a destination or version, first download the script, then choose an invocation:

```bash
curl -fsSLo install.sh https://github.com/biulight/shine/releases/latest/download/install.sh
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=2.10.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

The default destination is `%LOCALAPPDATA%\Programs\shine\shine.exe`. The installer does not change
the user `PATH`. If it reports the directory is missing, follow its instructions to add it to your
user `PATH`, open a new terminal, and run `shine --version`.

To choose a destination or version, first download the script, then choose an invocation:

```powershell
Invoke-WebRequest https://github.com/biulight/shine/releases/latest/download/install.ps1 -OutFile install.ps1
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "2.10.0"; .\install.ps1
```

## Install preview

These commands install preview to the same destination as stable releases, replacing any existing
Shine binary there. Both preview installer scripts select preview by default; an existing
`SHINE_VERSION` environment variable overrides that default. Preview builds are prereleases and may be
less stable. Check the installed build with `shine --version`.

To install the rolling preview build directly on macOS or Linux:

```bash
curl -fsSL https://github.com/biulight/shine/releases/download/preview/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/biulight/shine/releases/download/preview/install.ps1 | iex
```

Update preview with `shine self upgrade --channel preview`, or return to the latest stable release
with `shine self upgrade --channel stable`. Installing preview does not persist an update-channel
setting: `shine self upgrade` without a channel still selects stable.

## Install with cargo-binstall

With [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) installed, download a prebuilt
Shine binary from the official GitHub Releases:

```bash
cargo binstall shine-cli
shine --version
```

The package is named `shine-cli`; the installed command is `shine`. The default destination is
`$CARGO_HOME/bin` (usually `~/.cargo/bin`); make sure it is in `PATH`.

Official binaries cover x86_64 and aarch64 on macOS, Linux with glibc, and Windows (MSVC builds).
Shine currently publishes no Linux musl binaries.

By default, if no suitable official binary is found, cargo-binstall also checks the external
[cargo-quickinstall](https://github.com/cargo-bins/cargo-quickinstall) build service before falling
back to compiling from source. To use only official prebuilt binaries and fail if none is available, use:

```bash
cargo binstall shine-cli --strategies crate-meta-data
```

Run `cargo binstall shine-cli` again to upgrade through cargo-binstall. The moving `preview`
channel is available through `shine self upgrade --channel preview` instead.

## Install from source

With **Rust 1.90 or later** installed, you can install from crates.io:

```bash
cargo install shine-cli
```

To build from the Shine source repository, run `cargo build --release`. The binary is written to
`target/release/shine`.

## Upgrade Shine

Choose one update command:

| Operation | Command |
| --- | --- |
| Upgrade to the latest stable release | `shine self upgrade` |
| Reinstall latest stable; also return from preview | `shine self upgrade --channel stable` |
| Update to the rolling preview | `shine self upgrade --channel preview` |

`--channel stable` reinstalls even when the installed version is the same or newer.

`preview` is a continuously updated prerelease channel and is not included in routine automatic
update checks. `shine update` checks both installed configuration and stable Shine releases.

<span id="upgrade-to-shine-28" />
<span id="upgrade-to-shine-272" />
<span id="upgrade-to-shine-271" />
<span id="upgrade-to-shine-27" />
<span id="upgrade-to-shine-26" />
<span id="retire-an-older-local-http-service" />

Before upgrading, read [Upgrades and migration](./guides/upgrading.md) for actions that apply to your
current version and features. See [Release notes](https://github.com/biulight/shine/releases)
for new features and fixes.

Use `shine self install --dest <PATH>` to copy Shine to a chosen path. Relative paths start from the
current directory; add the destination's directory to `PATH`. Later self-upgrades also update this
copy. If Shine warns that the copy could not be updated, it may still be an older version. Resolve
the reported problem, then rerun the install command using the upgraded Shine executable.

On Unix, installing or syncing to a protected location requests `sudo` authorization. However,
Shine cannot elevate itself to upgrade when running from a protected directory. To use self-upgrade,
install and run Shine from a user-writable directory. On Windows, installing or upgrading in a
protected location requires a terminal with the necessary permissions.

## Uninstall Shine

Run `shine self uninstall --dry-run` from the account and configuration you want to clean up.
Review the listed changes. Run `shine self uninstall` to approve them interactively, or
`shine self uninstall --yes` to accept them without a prompt.

Shine uses each domain's uninstall rules for managed App files, Shell configuration, Sys items
and environment proxies. For the default account configuration, it also checks for exact Sys
startup blocks and the retired HTTP service. It keeps personal settings, tasks, external presets,
encryption identities, custom Sys profiles and software installed by bootstrap. The configuration
directory can still contain those files, caches and locks. The preview lists any trust grants
and broker policies scheduled for removal. If Shine cannot inspect the account's service
manager, retry from a session that can. If it finds a legacy service while using a custom
configuration directory, review `shine clear serve` separately.

If an interrupted operation or a modified managed file blocks cleanup, follow the affected resource's
recovery or uninstall instructions and rerun the command. If a Shell startup block points to
another Shine configuration or contains edits, review the reported file. Only the exact loader
for the selected configuration can be removed automatically. Earlier successful steps stay applied.
Shine removes program copies last, and only when it can confirm that it installed them and they
remain unchanged. A changed or different-version copy needs separate review.

For a Cargo installation, use the Cargo uninstall command printed by Shine. For a manual download
or an older installation without a matching receipt, remove the executable through its original
installation channel after managed cleanup. On Windows, close running instances and invoke
`self uninstall` from a separate copy of the same build with the same configuration. The separate
copy remains available after cleanup.

Next: [Complete your first preset installation](./quick-start.md).
