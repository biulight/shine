---
title: Upgrade from Shine 1.x
sidebar_position: 1
---

# Upgrade from Shine 1.x

Shine 2.0 is currently a release candidate. The stable update channel remains on 1.8.x, so trying
the RC is an explicit choice and does not change routine stable update checks.

## Install the exact RC

On macOS or Linux, download the matching installer and select the exact version:

```bash
curl -fsSLO https://github.com/biulight/shine/releases/download/v2.0.0-rc.1/install.sh
SHINE_VERSION=2.0.0-rc.1 sh install.sh
```

On Windows PowerShell:

```powershell
irm https://github.com/biulight/shine/releases/download/v2.0.0-rc.1/install.ps1 -OutFile install.ps1
$env:SHINE_VERSION = "2.0.0-rc.1"; .\install.ps1
```

Or install the exact crate version with Rust 1.88 or later:

```bash
cargo install shine-cli --version 2.0.0-rc.1
```

`shine self upgrade --channel preview` follows the continuously replaced preview build; it is not
the reproducible RC. Reinstall 1.8.x explicitly if you need to return to the stable series.

## Review before mutation

Install, upgrade, uninstall, generator refresh, artifact, and managed Sys operations now render a
snapshot-bound Plan. Interactive approval defaults to **No**. Review its steps, permissions, and
blockers, then approve it interactively or use `--yes` in an attended automation:

```bash
shine app upgrade <CATEGORY>
shine app upgrade <CATEGORY> --yes
```

`--yes` skips only the prompt. It does not skip Plan rendering, permission checks, or validation
against a fresh snapshot immediately before mutation.

## Re-establish external-code trust

The broad 1.x `allow_app_hooks` and `allow_sys_code` settings are retired, ignored, and removed on
the next configuration save. They are deliberately not converted into grants. External App, Shell,
and Sys executable targets require target-scoped trust bound to their source layer, code digest,
capability, and declared permissions:

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

Changing the external code or its requested permissions invalidates the old grant and requires a
new review.

## Generator and environment changes

Read-only status and info commands no longer execute App generators by default. Use
`--run-generators` when you intentionally want generator code to run. Lifecycle commands evaluate
only generators required by the selected operation and show their permissions in the Plan.

Hook and generator environments are narrowed to declared inputs. Each required environment source
must also be listed in the target's permission declaration; undeclared values are not inherited
from the parent process. Secret values never appear in a Plan or trust record.

## Sys profile and state migration

`shine upgrade` no longer changes Sys profile activation as a side effect. Manage it explicitly:

```bash
shine sys profile status
shine sys profile enable
shine sys profile disable
```

Inspect legacy runtime and environment state before applying its migration:

```bash
shine state migrate --dry-run
shine state migrate
```

Legacy App, Shell, and Sys manifests remain readable. Shine updates a manifest to the current
schema only after the associated mutation succeeds. Existing 1.8 Shell launchers without a receipt
can be planned and uninstalled directly; reinstalling first is not required. Modified or foreign
launchers and user-owned files are preserved and reported instead of overwritten.

## Recover interrupted operations

Journaled mutations stop later writes until their recovery Plan is reviewed. Use the command for
the affected lifecycle:

```bash
shine app recover
shine shell recover
shine sys recover
```

Recovery restores or removes only fingerprint-matched resources. Changed destinations, backups,
or rollback files block recovery and remain untouched for manual review.

## Update external Presets

External Presets must declare permission schema v1 for each executable target. A missing or invalid
declaration is a blocker, not an implicit broad grant. Authors should run the static and fixture
gates before distribution:

```bash
shine preset schema
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
shine preset test <CATEGORY>
shine preset pack <CATEGORY> --output <FILE>
```

Please report RC compatibility problems at the
[Shine issue tracker](https://github.com/biulight/shine/issues), including the rendered Plan and
platform but excluding secrets and private file contents.
