---
title: Upgrade from Shine 1.x
sidebar_position: 1
---

# Upgrade from Shine 1.x

This guide covers upgrading an existing 1.x installation to Shine 2.10. Review the approval
and recovery changes below, plus the applicable configuration and script changes in
[Upgrades and migration](./upgrading.md).

## Install Shine 2.10

Back up your configuration and presets, then use the existing installation to install the latest
stable release:

```bash
shine self upgrade --channel stable
```

If your older version cannot upgrade itself, follow the platform steps in
[Installation and upgrades](../installation.md).

## Review before mutation

Install, upgrade, uninstall, generator refresh, artifact, and managed Sys operations now show a
Plan before making changes. Interactive approval defaults to **No**. Review its actions,
permissions, and any issues blocking execution, then approve it interactively or use `--yes` in an attended automation:

```bash
shine upgrade app/<CATEGORY>
shine upgrade app/<CATEGORY> --yes
```

`--yes` skips only the prompt. The Plan, permission checks, and final validation still run.

## Re-establish external-code trust

The old `allow_app_hooks` and `allow_sys_code` settings are retired and do not become new grants
automatically. For external App, Shell, or Sys presets, approve the listed code for one operation
interactively, or review and save a trust grant for automation:

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

Running external code with `--yes` requires an existing grant. Changes to code or sources need a
new review. `--development` also trusts later code edits from the same local source; use it only
when you accept that scope. Shell live mode requires this authorization. See
[Custom presets](./custom-presets.md) for details.

## Generator and environment changes

Ordinary read-only checks do not run App generators by default. To check generated results, use
`shine info` or `shine update` with `--run-generators`; external generators need a trust grant
before this check.

If you maintain custom hooks or generators, convert old string `env` lists to structured inputs
with `source`, optional `target`, and `sensitivity`. Scripts receive only declared configuration
inputs, without inheriting undeclared environment values. Follow
[Declare runtime environment inputs](./custom-presets.md#declare-runtime-environment-inputs).

## Sys profile and state migration

`shine upgrade` no longer changes Sys profile activation as a side effect. Manage it explicitly:

```bash
shine sys status
shine sys profile enable <ITEM>
shine sys profile disable <ITEM>
```

Preview legacy state and encryption-configuration migration before applying it:

| Operation | Command |
| --- | --- |
| Preview legacy-state migration | `shine state migrate --dry-run` |
| Apply after reviewing the preview | `shine state migrate` |

Legacy App, Shell, and Sys installations remain readable and can be upgraded or removed without
reinstalling first. Modified or foreign command entries and user-owned files are preserved and
reported instead of overwritten.

## Recover interrupted operations

An interrupted managed operation may stop later writes until recovery is reviewed. Use the command
for the affected resource type:

```bash
shine app recover
shine shell recover
shine sys recover
```

Recovery acts only on files that have not changed since the interruption. Changed destinations,
backups, or recovery files remain untouched for manual review.

## Update external Presets

Review active 1.x external sources and overlays before the first configuration upgrade:

```bash
shine preset migrate --dry-run
shine preset migrate
```

For automation after reviewing the same source:

```bash
shine preset migrate --yes
```

You can also pass a repository, category directory, or `shine.toml`. Review the diff before
confirming. Migration backs up and updates metadata without changing scripts, data files,
environment values, installed resources, or trust. For a Git-managed overlay, migrate and validate
the upstream checkout, commit and push the changes, then run `shine preset pull` on each device.

If a preset is incompatible, resolve the reported issues before upgrading installed presets.
Custom Sys v1 presets need the manual [system preset migration](./sys-preset-v2-migration.md).

If you maintain external presets, validate the converted source:

```bash
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
```

Resolve errors, then review and renew external-code trust as described above. Permission
declarations alone do not authorize code. See [Custom presets](./custom-presets.md) for further
authoring and validation steps.

Please report upgrade compatibility problems at the
[Shine issue tracker](https://github.com/biulight/shine/issues), including the rendered Plan and
platform but excluding secrets and private file contents.
