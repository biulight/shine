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
Linux musl binaries are not provided. If no suitable binary is found, cargo-binstall may try a
third-party binary or compile from source. To require an official prebuilt binary, use:

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

Explicit `--channel stable` also permits reinstalling the same version or installing an older
stable release. Its success message reports the installed and previous versions without claiming
the version increased.

If `shine self upgrade` fails, it returns exit code 1 and reports the failure context and underlying
cause on stderr. Use that diagnostic to distinguish connection, archive, and installation errors.

`preview` is a continuously updated prerelease channel and is not included in routine automatic
update checks. `shine update` checks both installed configuration and stable Shine releases.

Stable releases are available through `latest` and the stable channel. Before upgrading an existing
1.x installation, read [Upgrade from Shine 1.x](./guides/upgrade-to-2.md) for the compatibility changes.

Shine 2.5 adds [temporary secret files](./guides/environment.md#workspace-secret-files) for build
credentials. Existing version 2 workspaces remain supported. Before adopting workspace version 3,
upgrade every client that reads it to Shine 2.5 or later; older clients reject the new format.
On Windows, new environment proxies require names without executable extensions (for example,
`gh` rather than `gh.exe`); existing suffixed proxies can still be uninstalled.

When upgrading from 2.3.x or earlier, review the
[environment-layer migration and rollback steps](./guides/environment.md#manual-upgrade-and-rollback-for-env-layering).
Version 2.4.0 changes which layer wins when keys overlap and replaces env write `--force` with explicit
target options. Keep private backups and check important projects with `shine env explain KEY`.

`shine self install --dest <PATH>` resolves a relative path from the current directory and remembers
the absolute destination, so later upgrades sync the same installed copy from any directory.
If the directory is not on PATH, the printed POSIX shell example preserves special characters in
the directory name. `self install` result paths escape terminal controls; a directory containing
controls gets a manual PATH instruction using the original directory instead of a copyable example.
If an upgrade succeeds but syncing this separate copy fails, Shine prints a
warning with the cause to stderr; the upgrade still returns success. Sync result paths and warning
details escape terminal controls without changing the recorded destination.

On Unix, if `shine self install` copies the binary to a location the current user cannot modify,
Shine interactively requests authorization and completes the installation through `sudo`. After a
successful `shine self upgrade`, the same behavior applies when syncing a separate recorded install
destination. Shine cannot currently elevate to replace the running binary itself when that binary
is in a protected directory; install and run it from a user-writable location instead. On Windows,
installing or upgrading in a protected location requires a terminal with the necessary permissions.

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
   of safety. Ordinary text groups repeated gaps; detailed views and lint JSON retain diagnostics.
   See [App code checks](./guides/custom-presets.md#check-app-code-before-execution).
3. **Update report consumers and renew approvals.** Run `shine preset schema` after upgrading.
   Lifecycle Plan/approval/review and author Plan reports use schema 3; lint and fixture reports
   use schema 2. Fixture input schema 1 remains supported. Obtain a new review instead of reusing
   an old approval; `--deny-warnings` still counts every warning despite grouped text.
   Rust integrations using `shine-core` must also adapt direct struct initializers and exhaustive
   patterns to new analysis fields, including those in `PlanV1` and `InspectionReportV1`, then
   rebuild. Serialized schema compatibility does not imply Rust source compatibility.

Installed launchers, receipts, recovery records and `shine env run --with SOURCE=TARGET` keep their
existing formats. Downgrading the binary does not undo metadata conversion: retain the original
source or migration backup if you need to return to an older client.

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
