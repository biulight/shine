---
title: Migrate system presets to v2
sidebar_position: 4
---

# Migrate system presets to v2

This page is for users maintaining **custom v1 system presets**. If you only use the system presets
built into Shine 2, no manual migration is needed; see [Initialize and manage a system](./system-init.md).

Migration changes `shine.toml` and scripts in your preset directory, not installed software. Do not
delete or manually rewrite the run records in `~/.shine/sys-manifest.toml`.

The example below migrates a Neovim item in `./my-presets/sys/macos/`. Replace this with your actual
path. For Ubuntu, use `sys/ubuntu/` and `--platform linux`; for Windows, use `sys/windows/` and
`--platform windows`. The example's Homebrew installation applies only to macOS.

## 1. Locate the preset to change

Before editing, preserve the old preset in version control or a separate copy, then inspect it:

```bash
shine preset migrate ./my-presets/sys/macos --dry-run
```

A `sys_v1_manual_migration_required` report with a nonzero exit code means you need to migrate
manually using the steps below. Shine cannot automatically split the old `init.sh` or `init.ps1`
into individual installers; rerunning the migration command will not do that work.

If you do not know which preset is active, run `shine preset migrate --dry-run` and check the paths
in its report. For a Shine-managed Git overlay, edit its upstream checkout rather than the local mirror.

## 2. Migrate one installation item first

Suppose a branch in the old `init.sh` calls `brew install neovim`. In v2, describe that installation
directly in `sys/macos/shine.toml`:

```toml
version = 2
description = "My macOS development tools."
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install Neovim with Homebrew."

[items.detect]
kind = "command"
command = "nvim"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew"
package = "neovim"

[profiles.recommended]
items = ["neovim"]
```

This is a complete example containing one item. When migrating an existing file, keep its other
items and their IDs and convert them individually; do not replace the whole configuration with this example.

- `detect` tells Shine how to check whether software is present; here it checks the `nvim` command.
- `install` tells Shine how to install missing software. This example uses Homebrew, which must be
  available before the actual installation.
- A fixed package provider needs no empty `permissions` table. Shine derives its operation identity
  from typed metadata.
- `[profiles.recommended]` selects the items in that group. Check that IDs in existing groups still
  refer to valid items.

For Ubuntu or Windows, use the appropriate `apt` or `winget` provider and verify the actual package
name in that package manager; do not assume the Homebrew package name also applies.

### When installation needs a custom script

If a package manager cannot perform an item's installation, move its logic out of the old script
into `install/<item>.sh` or `install/<item>.ps1`. Change that item's `install` to
`kind = "script"`, `path = "install/<item>.sh"` (use `.ps1` for Windows).

The script handles only that item, reports success or failure with a normal exit code, and still
needs a corresponding `detect`. Core automatically marks it as unisolated. Add optional author
capability statements only when they help review; keep environment inputs and Administrator
requirements explicit. See [capability statements](./custom-presets.md#declare-permissions).

After converting every item, remove the old shared entry point and its old status-output and
update-check logic. Third-party software updates belong to its package manager or upstream tool;
`shine sys bootstrap` only ensures that software is installed.

## 3. Migrate shell configuration if needed

Skip this step if the old preset does not change shell configuration.

Move software-specific PATH entries, environment variables, aliases, or initialization commands
into that item's `[[items.shell]]`. Longer scripts can live in `profile/<item>.sh` or `.ps1` and be
referenced through `fragment`. Keep only OS-wide content in `profile/base.pre.*` and `profile/base.post.*`.

See [Author a system bootstrap item](./custom-presets.md#author-a-system-bootstrap-item) for forms
and examples. Check that each software-specific integration belongs to its item and no longer runs
a second time from a shared file.

## 4. Validate the local files

```bash
shine preset validate ./my-presets/sys/macos
shine preset plan ./my-presets/sys/macos --platform macos
```

First fix errors reported by `validate`, such as missing files, invalid capability statements, or invalid
configuration. Then review the installation targets, required permissions, and shell configuration steps in
`plan` and check that they match your intent.

Neither command installs software or executes preset scripts. `preset plan` uses a simulated
environment, not your machine's installation state. Missing simulated trust, commands, or
administrator conditions can also block its report. Distinguish configuration errors from
environment requirements; do not broaden permissions just to make the report pass.

## 5. Make Shine use the migrated preset

If you edited the source already configured in Shine, no relinking is needed. If you migrated
another checkout, choose the appropriate method below and pass the root containing `sys/`,
not `sys/macos/`:

For a complete preset repository, select it as the external source:

```bash
shine preset link ./my-presets
```

For a repository overriding selected built-in presets, use the overlay instead:

```bash
shine preset overlay link ./my-presets
```

The chosen command replaces its corresponding source setting.
For a Git-managed overlay, first publish the upstream changes to its configured branch, then run
`shine preset pull`. See [Custom presets](./custom-presets.md) for source configuration details.

On the target operating system, inspect the item Shine actually reads:

```bash
shine sys list
shine sys info neovim
shine sys bootstrap neovim --dry-run
```

Check that the list and details contain your migrated item and that the preview shows the intended
installation command and shell configuration. If it still shows old content, check the source path
and overlay overrides first.

The package-only example above does not need an additional code trust grant. If you added an
external installer or executable shell content (`eval`, `source`, fragments, or shared profile
scripts), review that code, then inspect and grant trust for the corresponding item:

```bash
shine trust inspect sys/neovim
shine trust grant sys/neovim
```

A capability statement is optional, unverified review context; a trust grant records your review
and permission to execute external code. Package-only items need no empty declaration. Snapshot
trust binds the complete effective Sys OS snapshot and requires review after any file, source-layer,
or author/env/admin statement change.
Run the installation preview again after granting trust. During ongoing local authoring,
`--development` may accept code
edits from the same enrolled source; statement and source changes still require review.

Migration preparation is complete when local validation has no errors, Shine reads the correct
source, and the installation preview matches your intent. When ready to install, run
`shine sys bootstrap neovim` and review the plan before confirming. Continue with
[Initialize and manage a system](./system-init.md) for everyday use.
