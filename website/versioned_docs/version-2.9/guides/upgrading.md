---
title: Upgrades and migration
sidebar_position: 0
slug: /upgrading
---

# Upgrades and migration

Run `shine --version` before upgrading and read every item that applies to your starting version
and the features you use. When skipping versions, these requirements apply cumulatively;
the groups below do not require installing each intermediate release. Prepare the applicable
configuration or script changes below before [updating the program](../installation.md#upgrade-shine).

When upgrading from **1.x**, also read [Upgrade from Shine 1.x](./upgrade-to-2.md).

<span id="upgrade-to-shine-28" />

This page covers advance preparation and manual migration. For new features and fixes, see
[Release notes](https://github.com/biulight/shine/releases).

## From 2.6.x or earlier: Preset environment inputs {#upgrade-to-shine-27}

When upgrading from 2.6 or earlier, review these compatibility changes before updating external
Presets or automation. Source builds now require Rust 1.90; prebuilt binaries do not require Rust.

**Convert legacy Preset environment inputs.** Shell Bun commands and App generators, hooks and
artifacts require structured `env` entries containing `source`, optional `target`, and
`sensitivity`. Legacy string arrays are rejected. Use Shine 2.7 or later to preview the conversion
before applying it; the migrator automatically backs up the metadata it changes:

| Operation | Command |
| --- | --- |
| Preview this source's migration | `shine preset migrate ./my-presets --dry-run` |
| Apply after reviewing the preview | `shine preset migrate ./my-presets` |
| Validate the migrated metadata | `shine preset validate ./my-presets` |
| Check authoring issues | `shine preset lint ./my-presets` |

The migrator needs explicit sensitivity from the old declarations; missing or conflicting
classifications require manual correction. Migrate Git-managed overlays in their upstream
checkout, then pull again. Migration backs up metadata but does not execute scripts, decrypt
values, or renew trust. Review changed external snapshots before normal install/upgrade.
See [runtime environment inputs](./custom-presets.md#declare-runtime-environment-inputs).

## From 2.5.x or earlier: uninstall scripts {#upgrade-to-shine-26}

Shine 2.6 removes `--purge` from `uninstall`, `app uninstall`, and `shell uninstall`.
Remove that option from scripts; commands that still contain it fail before loading configuration.

## From 2.3.x or earlier: replace HTTP service dependencies {#retire-an-older-local-http-service}

If applications still reference the old Shine HTTP service, switch them to local files before
upgrading. Replace `SHINE_APP_HTTP_DIR` in scripts; application artifacts can keep persistent
output in `SHINE_STATE_DIR` or `SHINE_CACHE_DIR`. Upgrading does not remove files in the old
`http/` directory. After updating references, follow the
[old service cleanup steps](./system-init.md#retire-legacy-http-service).

## From 2.3.x or earlier: environment layering

When upgrading from 2.3.x or earlier, follow the
[environment-layer migration and rollback steps](./environment.md#manual-upgrade-and-rollback-for-env-layering).
Update scripts that use env write `--force`, and run
`shine env explain KEY` in important projects to check which value takes effect after upgrading.
