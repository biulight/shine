---
title: Upgrade from Shine 1.x
sidebar_position: 1
---

# Upgrade from Shine 1.x

This guide covers upgrading an existing 1.x installation to Shine 2.7. Review the lifecycle security
and recovery changes below, plus the [2.7 migration steps](../installation.md#upgrade-to-shine-27).

Interactive lifecycle Plan confirmation can authorize the listed external code for that operation only,
without saving trust. Automation and `--yes` require an existing grant. Shell live always requires
Development trust; explicit `--run-generators` inspection also requires a grant.

## Install Shine 2.7

On macOS or Linux, install the latest stable release:

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

Or install from crates.io with Rust 1.90 or later:

```bash
cargo install shine-cli
```

An existing Shine installation can switch explicitly to the stable channel:

```bash
shine self upgrade --channel stable
```

`shine self upgrade --channel preview` follows the continuously replaced preview build instead of
the stable release.

## Review before mutation

Install, upgrade, uninstall, generator refresh, artifact, and managed Sys operations now show a
Plan before making changes. Interactive approval defaults to **No**. Review its actions,
permissions, and blockers, then approve it interactively or use `--yes` in an attended automation:

```bash
shine upgrade app/<CATEGORY>
shine upgrade app/<CATEGORY> --yes
```

`--yes` skips only the prompt. The Plan, permission checks, and final validation still run.

## Re-establish external-code trust

The broad 1.x `allow_app_hooks` and `allow_sys_code` settings are retired, ignored, and removed on
the next configuration save. They are deliberately not converted into grants. External App, Shell,
and Sys executable targets need human confirmation in the lifecycle Plan, or persistent
target-scoped trust for automation:

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

Changing the effective category content invalidates a snapshot grant and requires a
new review. Preset authors may explicitly use `shine trust grant <TARGET> --development` to trust
later code edits from the same local source; source, target, or capability changes still require authorization.

## Generator and environment changes

Read-only status and info commands no longer execute App generators by default. Use
`--run-generators` when you intentionally want generator code to run. Lifecycle commands evaluate
only generators required by the selected operation and show their permissions in the Plan.

Hook and generator environments are narrowed to their own structured `env` inputs. Declare
`source`, optional `target`, and `sensitivity` together; environment permissions are derived from
these inputs, without a duplicate permission entry. Undeclared values are not inherited from the
parent process. Secret values never appear in a Plan or trust record. Convert legacy string arrays
with the [explicit migration steps](custom-presets.md#declare-runtime-environment-inputs).

## Sys profile and state migration

`shine upgrade` no longer changes Sys profile activation as a side effect. Manage it explicitly:

```bash
shine sys status
shine sys profile enable <ITEM>
shine sys profile disable <ITEM>
```

Inspect legacy runtime and environment state before applying its migration:

```bash
shine state migrate --dry-run
shine state migrate
```

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
# For automation after reviewing the same source:
shine preset migrate --yes
```

You can also pass a repository, category directory, or `shine.toml`. Review the diff before
confirming. Migration backs up and updates metadata without changing scripts, data files,
environment values, installed resources, or trust. For a Git-managed overlay, migrate and validate
the upstream checkout, commit and push the changes, then run `shine preset pull` on each device.

Sys v1 dispatchers require manual migration. Follow the reported target and validate the updated
source with `shine preset validate <PATH>` and `shine preset plan <PATH> --platform <PLATFORM>`.
Missing optional capability tables alone do not require metadata conversion. Supported App code
still needs accurate declarations for statically identified behavior; see
[App code checks](custom-presets.md#check-app-code-before-execution).
A managed Git overlay remains read-only; its report points back to the upstream checkout.

Trust enrollment applies to external App hook/generator/artifact code, Sys bootstrap/profile code,
and every external Shell command, independent of capability-table contents. After validation, use
the canonical target—or `shine trust inspect preset` for the current batch—and grant only after
accepting the rendered scope. Older grants require review again; live Shell deployment requires development trust.

If Shine reports an incompatible Preset, run `shine preset migrate --dry-run` for remediation
steps. Resolve compatibility errors before upgrading installed presets.

Permission tables are optional author capability statements. Explicit tables must use supported
schema v1 or v2 and pass validation; neither a missing table nor a valid declaration grants code
trust. Authors should run the static and fixture gates before distribution:

```bash
shine preset schema
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
shine preset test <CATEGORY>
shine preset pack <CATEGORY> --output <FILE>
```

Please report upgrade compatibility problems at the
[Shine issue tracker](https://github.com/biulight/shine/issues), including the rendered Plan and
platform but excluding secrets and private file contents.
