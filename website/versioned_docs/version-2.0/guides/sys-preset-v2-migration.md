---
title: Migrate system presets to v2
sidebar_position: 4
---

# Migrate system presets to v2

System presets now use `version = 2`. Version 1 manifests are rejected before Shine runs detection,
an installer, elevation, or profile writes. Existing `~/.shine/sys-manifest.toml` run records remain
readable; this migration changes preset authoring, not recorded software ownership.

Start with `shine preset migrate --dry-run`, or pass the Sys repository, category, or
`shine.toml`. The report identifies `sys_v1_manual_migration_required`, preserves every dispatcher
and script byte, and exits nonzero: a v1 dispatcher is opaque platform-wide code and cannot be
safely split automatically. `shine preset migrate` never changes runtime manifests or trust grants.

For each init item, replace the platform-wide `init.sh` or `init.ps1` dispatcher with both:

- a read-only `detect` declaration (`command`, `path`, or `any`); and
- an `install` declaration using a fixed package provider or one item-specific script under `install/`.

Set `version = 2` at the manifest root. Move software-specific shell content into `[[items.shell]]`
or an item-owned `profile/<item>.*` fragment. Base profile files may contain only platform-wide
content. Remove status/update wire-protocol output and update-check dispatches: users upgrade
third-party software with its package manager or upstream tool.

External install scripts, base profile files, fragments, `eval`, and `source` require a current
target-scoped `shine trust grant sys/<ITEM>`. Static detection, provider metadata, PATH,
environment, and aliases do not need a grant.

Every `[[items]]` target also carries permission schema v1. Fixed providers and managed targets are
already bounded by typed metadata; item scripts conservatively declare their Preset-relative
executable path plus reviewed command, network, administrator, environment, and system identities.
This declaration is validated but does not grant trust or make opaque code statically provable.

Start from `shine preset copy sys/<os>`, validate and plan it, then inspect any external code before
granting trust:

```bash
shine preset validate <PATH>
shine preset plan sys/<os> --platform <macos|linux|windows>
shine trust inspect sys/<ITEM>
shine trust grant sys/<ITEM>
shine sys list
shine sys info <ITEM>
shine sys bootstrap <ITEM> --dry-run
```
