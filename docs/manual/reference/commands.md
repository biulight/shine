---
title: Command reference
sidebar_position: 1
---

# Command reference

This page describes the current development version (Next). Run `shine <COMMAND> --help` for
the options supported by your installed version. The task guides explain complete workflows;
this page is a compact command index.

Command failures are written to stderr. Set `NO_COLOR=1` to disable colored output.

## Targets

Use complete targets in scripts and documentation:

- `app/<category>` for application configuration;
- `shell/<category>` or `shell/<category>/<command>` for shell presets;
- `sys/<item>` for managed system items.

A bare app or shell category is accepted when it is unambiguous. Bare shell command names are for
inspection only.

Target names are matched literally. Quote names containing leading or trailing whitespace;
Shine does not strip that whitespace. An entirely blank target is rejected.

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

## Top-level commands

| Command | Purpose |
| --- | --- |
| `shine init [--yes]` | Create `shine.config.toml` for the current project |
| `shine list [--available [KIND]]` | List installed resources or browse the `app`, `shell`, and `sys` catalogs |
| `shine info <TARGET> [--diff] [--verbose]` | Inspect an available or installed target |
| `shine install <TARGET> [--verbose]` | Install or repair an app or shell target |
| `shine uninstall <TARGET> [--verbose]` | Uninstall an app or shell target |
| `shine update [TARGET]` | Check managed content and Shine releases without applying changes |
| `shine upgrade [TARGET]` | Apply all or selected managed updates |
| `shine shell ...` / `shine app ...` / `shine sys ...` | Use resource-specific operations |
| `shine preset ...` | Create, validate, and manage Preset sources |
| `shine env ...` | Manage values, workspace environments, proxies, and secrets |
| `shine ssh ...` / `shine local ...` | Open SSH sessions and transfer files |
| `shine task ...` / `shine run <NAME>` | Save and run personal commands |
| `shine self ...` | Install or upgrade the Shine binary |
| `shine state migrate` | Preview or apply supported legacy-state migrations |
| `shine state check` | Check for maintenance actions now without applying them |
| `shine clear serve` | Stop and remove the retired HTTP service registration |
| `shine trust ...` | Review target-scoped trust for external Preset code |
| `shine completions ...` | Generate or install shell completions |
| `shine theme sync` | Print terminal-theme environment exports |

Every command accepts the global `--config-dir <PATH>` option.

Quote target names containing spaces or shell punctuation. For names beginning with `-`, put
options before `--` and the name after it, for example `shine app uninstall --dry-run -- -demo`.
Suggested commands use POSIX shell syntax on Unix and PowerShell syntax on Windows.

Before confirming a change, review the affected files, permissions, code sources, and execution
steps. Add `--verbose` for details; for upgrades, add `--full-plan` with `--verbose` to include
unchanged items. If relevant files or settings change after review, review the new plan.

Interactive confirmation can authorize listed external code for that operation without saving
persistent trust. Automation and `--yes` require existing trust for external code. `--yes` skips only the prompt,
not permissions or execution checks, and cannot be combined with `--dry-run`. Preset scripts are
not sandboxed.

## Shell presets

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine shell recover [--yes] [--verbose]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--yes] [--verbose]
```

- Preview installation or uninstall with `--dry-run`.
- `shell install --dry-run` lists link candidates, not a security Plan. It does not evaluate
  ownership conflicts, `--replace-managed` effects, trust, permissions, or profile changes;
  `--verbose` does not expand this preview. Run without `--dry-run` to review the Plan before approval.
- `shell list` shows the active presets, including overlay additions and overridden descriptions.
- `--replace-managed` may overwrite managed content changed after installation. Inspect the target
  with `shine info <TARGET> --diff` first.
- If Shine reports an interrupted operation, run `shine shell recover`. Changed files are preserved
  and may require manual review.
- Shell uninstall identifies retained launcher paths when ownership conflicts prevent removal.
  It has no `--force` override: inspect those files before deciding whether to move or remove them.
  A conflicting file may remain after Shine removes its installation record; check the reported paths.

See [Manage shell presets](../guides/shell-presets.md).

## Application presets

```text
shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine app refresh <CATEGORY> [FILE] [--force] [--yes] [--verbose]
shine app recover [--yes] [--verbose]
shine app uninstall [CATEGORY] [--force] [--dry-run] [--yes] [--verbose]
shine app artifact apply <APP_ID> [--yes] [--verbose]
shine app artifact remove <APP_ID> [--yes] [--verbose]
```

- `app info` and `update` do not run generators unless `--run-generators` is present.
- `app refresh` runs a generated-file refresh explicitly. `--force` permits replacing a
  user-modified managed destination. The destination must already be managed for that category and
  source file; `--force` cannot take over another source's destination.
  Its summary distinguishes `user-modified (kept)` from execution failures. Either leaves the
  refresh incomplete and returns a nonzero exit status; other selected files may still refresh.
- `app uninstall --force` may delete user-modified managed content. Always preview it with
  `--dry-run`.
- Use `shine app recover` when an interrupted App operation blocks later changes.

See [Manage application configuration](../guides/app-presets.md).

If a Preset source disappears, installed App files are preserved. Preview their removal with
`shine app uninstall <OLD_CATEGORY> --dry-run`, then remove `--dry-run` to proceed. Use the `app/`
or `shell/` prefix when category names conflict.

Uninstall normally preserves user-modified files. Unchanged managed files are removed and available
backups restored; JSON merge uninstall removes only managed keys. `--force` permits deleting local
changes. Missing source code cannot run teardown scripts. See the App guide for recovery and cleanup.

An incomplete App or Shell uninstall returns a nonzero exit status, even if some removals succeeded;
completed removals are not undone. Review the remaining conflicts before retrying. A successful
`--dry-run` only confirms that the preview completed, not that every resource can be removed.

## Status, updates, trust, and completions

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--full-plan] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine state check
shine trust inspect <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine trust grant <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM> [--development] [--yes]
shine trust list [--verbose]
shine trust revoke <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

Dynamic completion includes candidates from the selected Preset source and its active overlay.
Tab completion does not execute Preset code or grant trust.

`info` inspects files without repairing them. `state migrate --dry-run` previews supported legacy
state migrations; if configuration validation fails, correct the reported file before retrying.

If a release check fails, `update` still reports configuration updates. Retry the release check
with `shine update --refresh-release` after restoring connectivity; this option cannot be combined
with a target.

`update` normally checks generated files without running their generators. Add `--run-generators`
to evaluate their output. Fully built-in categories need no additional grant; a category containing
external or overlay files needs matching trust. Review the reported sources before granting it.

A failed or incomplete evaluation cannot establish that installed content is current. Even when
checking one file, a generator failure elsewhere in its App category can make the command exit
with status 1. Use `--verbose` for diagnostics or
[check the preset source](../guides/custom-presets.md#check-app-code-before-execution) with `preset lint`.

For external Shell presets in `snapshot` mode, `update --diff` includes shared category files,
not just the selected command script. `content: unchanged` does not rule out changes to helpers
or metadata. `update <TARGET>` shows these details by default. Displayed diffs are review previews,
not patches; line-ending-only changes can still require an update.

`update --verbose` includes managed system settings with no pending update. The label
`up-to-date (recorded)` means the desired settings match Shine's installation record, not that
Shine checked the live system. Resolve any missing sources or unchecked generators before
assuming everything is current.

If an installed App category or file disappears from the Preset source, `update --verbose` still
reports its retained files and offers an uninstall preview. Checking does not delete them.

`update` checks without applying changes. `upgrade` asks for approval when work is needed and
reports `Nothing to upgrade.` when no work or unresolved checks remain. Unchanged command contents
can still require updated configuration or shell integration. Review the plan before using `--yes`.

| Option | Effect |
| --- | --- |
| `--pull` | Update eligible Git-managed Preset sources before checking or upgrading |
| `--prune-stale` | Allow removal of unchanged managed App files no longer present in the Preset |
| `trust grant --development` | Authorize future code edits from the displayed local source |

Development trust is required for live Shell deployment. It covers only the approved target,
capabilities, and source directory/layer; it does not continuously review future code. Use
`trust list` to inspect grants and `trust list --verbose` for details. Listing does not renew them.
See [trust and capability declarations](../guides/custom-presets.md#declare-permissions).

Missing sources, user-modified files, conflicting command entries, missing required inputs or
administrator authorization, and untrusted external code require attention. Follow the reported
remedy; these conditions are not permission to overwrite files.

## System presets

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys recover [--yes] [--verbose]
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes] [--verbose]
shine sys profile enable <ITEM> [--dry-run] [--yes] [--verbose]
shine sys profile disable <ITEM> [--dry-run] [--yes] [--verbose]
shine sys apply [ITEM] [--dry-run] [--yes] [--verbose]
shine sys uninstall <ITEM> [--dry-run] [--yes] [--verbose]
```

`sys list --all` marks other operating systems as `catalog only` and shows apply hints only for
the current OS. `sys info`, `bootstrap`, `apply`, and `uninstall` always resolve items for the
current OS, including when another OS has an item with the same name.
Use `bootstrap` to ensure selected software and shell integration are present. Use `apply` and
`uninstall` for reversible managed system configuration. These operations may request administrator
access after you approve their plan. `--force-profile` may replace conflicting profile content, so
review a dry run first.

Preview changes with `--dry-run` before applying or removing system configuration. It cannot be
combined with `--yes`. Use `--yes` for non-interactive approval; it skips only the prompt, not the
plan, permission checks, or final validation. Interactive confirmation can authorize the listed
external code for one operation without saving trust. Running external code with `--yes` requires
an existing [trust grant](../guides/custom-presets.md).

Positional `bootstrap` IDs select items in order and ignore duplicates. Repeated `--item` is an
equivalent option, for example `shine sys bootstrap --item rust --item mise --yes`.
Positional IDs, `--item`, and `--preset` cannot be combined. Non-interactive runs without explicit
items or a profile use the preset's default profile. For IDs beginning with `-`, put options
before `--` and the ID after it, for example `shine sys apply --dry-run -- -custom`.

For managed system settings, `shine update` compares the desired configuration with installation
records; `shine upgrade` also checks the actual system resource. Windows split-DNS settings that
match both the record and desired configuration need no update or administrator access.
Use `shine upgrade shell/<CATEGORY>` to limit an upgrade to Shell presets.

See [Initialize and manage a system](../guides/system-init.md).

## Preset authoring and sources

```text
shine preset new <app|shell|sys> [--unrestricted] [--force]
shine preset schema [--format <text|json>]
shine preset validate [PATH] [--format <text|json>]
shine preset lint [PATH] [--format <text|json>] [--deny-warnings]
shine preset plan <CATEGORY> --platform <macos|linux|windows> [--format <text|json>]
shine preset test <CATEGORY> [--format <text|json>]
shine preset pack <CATEGORY> --output <FILE> [--force] [--format <text|json>]
shine preset migrate [PATH] [--dry-run] [--yes] [--format <text|json>]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset overlay info` reports the effective overlay, including a project's local overlay in
preference to an inherited Git overlay. With a project configuration, `SHINE_CONFIG_DIR` alone
does not override a directory selected by `preset link`; `SHINE_PRESETS` still takes priority.
`preset unlink` removes the saved setting. Environment overrides and inherited settings still
apply, so the next run may continue to use an external presets directory.
Use `validate`, `lint`, `plan`, and `test` before distributing a Preset. These commands inspect
it without installing it. Use `migrate --dry-run` to preview metadata changes, then review and
confirm before applying them. For a Git-managed overlay, migrate and validate its upstream checkout,
commit and push the changes, then run `shine preset pull` on each consuming device.

`--unrestricted` adds an optional author statement to a new template. It neither isolates script
execution nor grants trust or permission to run the code.

See [Customize presets](../guides/custom-presets.md) and
[Migrate system presets to v2](../guides/sys-preset-v2-migration.md).

## Environment values and secrets

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--global|--project|--overlay] [--env-file]
shine env get <KEY>
shine env explain <KEY> [--reveal]
shine env delete <KEY> [--global|--project|--overlay] [--env-file]
shine env run [--workspace <FILE>] [--mode <MODE>] [--file-group <NAME>]... [--dry-run] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env workspace export --format files [--workspace <FILE>] --mode <MODE> --file-group <NAME>... [--secret-broker] [--dry-run]
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--global|--project|--overlay] [--env-file]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]...
shine env secret seal --file-group <NAME>... [--workspace <FILE>] [--mode <MODE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]... [--remove-plaintext] [--dry-run]
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity init --phone [--recipient-type <tag|phone>] [--label <LABEL>] [--transport <auto|adb|qr>] [--adb-serial <SERIAL>]
shine env secret identity list
```

Scope options are mutually exclusive; the default is the active project, otherwise global.
`--env-file` selects that scope's `shine.env.toml`; `--overlay` requires a manual directory.
If a higher-priority source overrides a key, choose the intended write target explicitly and
check the effective source afterward. The old `--force` for `set`, `delete`, and `secret encrypt`
is no longer supported; use scope and file options instead.

`env explain --reveal`, `env list --reveal`, `env get`, `env secret decrypt`, exports with `--include-secrets`, and child
commands started by `env run` can expose plaintext. Run them only in a trusted terminal and never
put secrets directly in command arguments or documentation.

Broker policy commands are documented with the complete remote workflow in
[SSH sessions, secret brokering, and file transfer](../guides/ssh-transfer.md). For local values,
workspace encryption, command wrappers, and hybrid backends, see
[Manage environment variables and secrets](../guides/environment.md).

See [temporary secret files](../guides/environment.md#workspace-secret-files) for file-group configuration, sealing, persistent export, execution, and cleanup boundaries.

## Tasks and theme

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine theme sync [--auto] [--quiet]
```

Saved tasks execute the stored command, optionally from its stored working directory.

See [Tasks and application artifacts](../guides/tasks-and-serve.md) and
[Synchronize the terminal theme](../guides/terminal-theme-sync.md).

## Cleanup

### Maintenance reminders

Shine can remind you about maintenance during ordinary interactive use. Checks do not remove
services or files. To check immediately, including in scripts or when configuration cannot load, run:

```text
shine state check
```

Automatic reminders are skipped in noninteractive use and may appear on a later invocation.
If a check cannot finish, resolve the reported issue and retry `shine state check`.

### Remove the retired HTTP service

```text
shine clear serve
```

`clear` requires an explicit target; running it alone does not clean up everything. `clear serve`
stops and removes an older version's user service without loading Shine configuration or deleting
published files. Repeating successful cleanup is safe; service-manager errors are reported.
The `shine serve` namespace is removed. See
[Upgrades and migration](../guides/upgrading.md#retire-an-older-local-http-service).

`shine state migrate` remains the command for versioned runtime-state migrations.

## SSH and file transfer

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session] <HOST>
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Place Shine forwarding and broker options before the SSH destination. Direct secret forwarding
makes plaintext available to the remote session. File transfer is available only in POSIX remote
mode; preview overwrites with `--dry-run`.

See [SSH sessions, secret brokering, and file transfer](../guides/ssh-transfer.md).

## Shine installation and upgrades

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
shine self uninstall [--dry-run | --yes]
```

`stable` follows released versions. `preview` follows the replaceable preview build. See
[Installation and upgrades](../installation.md).
