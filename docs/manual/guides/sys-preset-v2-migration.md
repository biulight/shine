---
title: Migrate system presets to v2
sidebar_position: 4
---

# Migrate system presets to v2

System presets now use `version = 2`. Version 1 manifests are rejected before Shine runs detection,
an installer, elevation, or profile writes. Existing `~/.shine/sys-manifest.toml` run records remain
readable; this migration changes preset authoring, not recorded software ownership.

For each init item, replace the platform-wide `init.sh` or `init.ps1` dispatcher with both:

- a read-only `detect` declaration (`command`, `path`, or `any`); and
- an `install` declaration using a fixed package provider or one item-specific script under `install/`.

Set `version = 2` at the manifest root. Move software-specific shell content into `[[items.shell]]`
or an item-owned `profile/<item>.*` fragment. Base profile files may contain only platform-wide
content. Remove status/update wire-protocol output and update-check dispatches: users upgrade
third-party software with its package manager or upstream tool.

External install scripts, base profile files, fragments, `eval`, and `source` require a current
target-scoped `shine trust grant sys/<ITEM>`. Static detection, provider metadata, PATH,
environment, and aliases do not need a grant. Start from `shine preset copy sys/<os>` and validate
with:

Every `[[items]]` target also carries permission schema v1. Fixed providers and managed targets are
already bounded by typed metadata; item scripts conservatively declare their Preset-relative
executable path plus reviewed command, network, administrator, environment, and system identities.
This declaration is validated but does not grant trust or make opaque code statically provable.

```bash
shine sys list
shine sys info <ITEM>
shine sys bootstrap <ITEM> --dry-run
```
