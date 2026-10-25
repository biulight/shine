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
SHINE_VERSION=2.7.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

The default destination is `%LOCALAPPDATA%\Programs\shine\shine.exe`. The installer does not change
the user `PATH`. To choose a destination or version:

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "2.7.0"; .\install.ps1
```

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

After installation, Shine can download either the stable or preview build:

```bash
shine self upgrade
shine self upgrade --channel stable
shine self upgrade --channel preview
```

Without a channel option, Shine upgrades to the latest stable release. Use `--channel stable`
to reinstall the latest stable release, including when the installed version is the same or newer.

`preview` is a continuously updated prerelease channel and is not included in routine automatic
update checks. `shine update` checks both installed configuration and stable Shine releases.

Before upgrading from Shine 1.x, read [Upgrade from Shine 1.x](./guides/upgrade-to-2.md).

Before using [temporary secret files](./guides/environment.md#workspace-secret-files), upgrade all
clients that read the same workspace to Shine 2.5 or later. On Windows, use names without executable
extensions for new environment proxies, such as `gh` instead of `gh.exe`.

When upgrading from 2.3.x or earlier, follow the
[environment-layer migration and rollback steps](./guides/environment.md#manual-upgrade-and-rollback-for-env-layering).
Back up your configuration privately, update scripts that use env write `--force`, and run
`shine env explain KEY` in important projects to check which value takes effect after upgrading.

Use `shine self install --dest <PATH>` to copy Shine to a chosen path. Relative paths start from the
current directory; add the destination's directory to `PATH`. Later self-upgrades also update this
copy. If Shine warns that the copy could not be updated, it may still be an older version. Resolve
the reported problem, then rerun the install command using the upgraded Shine executable.

On Unix, installing or syncing to a protected location requests `sudo` authorization. However,
Shine cannot elevate itself to upgrade when running from a protected directory. To use self-upgrade,
install and run Shine from a user-writable directory. On Windows, installing or upgrading in a
protected location requires a terminal with the necessary permissions.

## Upgrade to Shine 2.7

When upgrading from 2.6 or earlier, review these compatibility changes before updating external
Presets or automation. Source builds now require Rust 1.90; prebuilt binaries do not require Rust.

1. **Convert legacy Preset environment inputs.** Shell Bun commands and App generators, hooks and
   artifacts require structured `env` entries containing `source`, optional `target`, and
   `sensitivity`. Legacy string arrays are rejected. Keep a private backup of your source, then use
   Shine 2.7 to preview the conversion before applying it:

   ```bash
   shine preset migrate ./my-presets --dry-run
   shine preset migrate ./my-presets
   shine preset validate ./my-presets
   shine preset lint ./my-presets
   ```

   The migrator needs explicit sensitivity from the old declarations; missing or conflicting
   classifications require manual correction. Migrate Git-managed overlays in their upstream
   checkout, then pull again. Migration backs up metadata but does not execute scripts, decrypt
   values, or renew trust. Review changed external snapshots before normal install/upgrade.
   See [runtime environment inputs](./guides/custom-presets.md#declare-runtime-environment-inputs).
2. **Review App code diagnostics.** Supported Bun code and local JS/TS dependencies are checked
   before execution. Definite declaration differences and analysis failures block affected work,
   even with existing trust or `--yes`. Fix the declarations or code; adding unrestricted-code
   metadata does not bypass the check. Coverage gaps remain advisory and are not a sandbox or proof
   of safety.
   See [App code checks](./guides/custom-presets.md#check-app-code-before-execution).
3. **Update automation and renew approvals.** If you consume structured reports or maintain preset
   tests, run `shine preset schema` after upgrading and follow the
   [authoring compatibility notes](./guides/custom-presets.md#check-app-code-before-execution).
   Obtain new approvals instead of reusing old ones. Rust integrations using `shine-core` may need
   source changes and recompilation for the updated public types.

Downgrading Shine does not undo metadata conversion. Keep the original source or migration backup
if you need to return to an older version.

## Upgrade to Shine 2.6

Shine 2.6 removes `--purge` from `uninstall`, `app uninstall`, and `shell uninstall`.
Remove that option from scripts; commands that still contain it fail before loading configuration.
Uninstall now includes internal cache cleanup in the Plan you approve. Preview removals with
`--dry-run`, especially before removing a whole App category or all Shell presets. External preset
sources are preserved. See [App removal](./guides/app-presets.md) and
[Shell removal](./guides/shell-presets.md) for scope and retained-file behavior.

Shine 2.6 also adds persistent secret-file export with
`shine env workspace export --format files --mode <MODE> --file-group <GROUP>`.
Unlike temporary files supplied to `env run`, exported plaintext stays on disk until you remove it.
Read the [secret-file guide](./guides/environment.md#workspace-secret-files) before use.

## Retire an older local HTTP service

Version 2.4.0 removes HTTP serving and the `SHINE_APP_HTTP_DIR` script variable. Use local
files for Surge and Clash Verge rules; application artifacts can keep persistent output in
`SHINE_STATE_DIR` or `SHINE_CACHE_DIR`.

If you installed the old background service, remove it after upgrading to 2.4.0:

```bash
shine clear serve
```

`shine clear serve` stops the old service and removes its user service registration. The entire
`shine serve` command is removed. Cleanup works even if the old Shine configuration can no longer
be loaded, and does not need the old binary or its `--config-dir`. Run it as the user who installed the service;
repeating it after successful removal is safe. Service-manager failures are reported so you can
correct them and retry.

Normal interactive commands remind you when they detect a retained service. To check immediately,
run `shine state check`; it only reports maintenance actions and works with unreadable configuration.

Before upgrading, the old binary's `shine serve uninstall` can also remove the service.

Upgrading does not remove existing files under the old runtime directory's `http/`. Keep any files
you still need and update application references before deleting them.

Next: [Complete your first preset installation](./quick-start.md).
