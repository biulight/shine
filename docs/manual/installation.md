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
Make sure `~/.local/bin` is in `PATH`, then verify the installation:

```bash
shine --version
```

To choose a destination or version:

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=1.7.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

The default destination is `%LOCALAPPDATA%\Programs\shine\shine.exe`. The installer does not change
the user `PATH`. To choose a destination or version:

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "1.7.0"; .\install.ps1
```

## Install from source

With **Rust 1.88 or later** installed, you can install from crates.io:

```bash
cargo install shine-cli
```

To build from the Shine source repository, run `cargo build --release`. The binary is written to
`target/release/shine`.

## Upgrade Shine

After installation, Shine can download either the stable or preview build:

```bash
shine self upgrade
shine self upgrade --channel stable
shine self upgrade --channel preview
```

`preview` is a continuously updated prerelease channel and is not included in routine automatic
update checks. `shine update` checks both installed configuration and stable Shine releases.

On Unix, if `shine self install` copies the binary to a location the current user cannot modify,
Shine interactively requests authorization and completes the installation through `sudo`. After a
successful `shine self upgrade`, the same behavior applies when syncing a separate recorded install
destination. Shine cannot currently elevate to replace the running binary itself when that binary
is in a protected directory; install and run it from a user-writable location instead. On Windows,
installing or upgrading in a protected location requires a terminal with the necessary permissions.

Next: [Complete your first preset installation](./quick-start.md).
