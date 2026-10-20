---
title: Command reference
sidebar_position: 1
---

# Command reference

This page reflects Shine 2.5.0. Run `shine <COMMAND> --help` for every option supported by the
installed version. The task guides explain complete workflows; this page is a compact command index.

## Targets

Use complete targets in scripts and documentation:

- `app/<category>` for application configuration;
- `shell/<category>` or `shell/<category>/<command>` for shell presets;
- `sys/<item>` for managed system items.

A bare app or shell category is accepted when it is unambiguous. Bare shell command names are for
inspection only.

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

Suggested App install/repair, upgrade, artifact, refresh, uninstall, Shell repair, and trust commands quote
names containing spaces or shell punctuation. Use these hints in a POSIX shell on Unix or in
PowerShell on Windows; the quotes preserve each original argument.
For positional names beginning with `-`, these hints put options before `--` and the names after it,
for example `shine app uninstall --dry-run -- -demo`.

Security Plans show user file paths explicitly and summarize installed files, Shine maintenance,
and associated backup/recovery files by purpose, with access types and path counts. Installation,
upgrade, uninstall, recovery, App refresh/artifact, and Sys bootstrap/profile/apply reviews accept
`--verbose` to expand file paths, steps, snapshot identities, and diagnostics in the displayed
review scopes.

Filesystem permission paths use familiar forms: `/etc/docker/daemon.json` for an absolute path,
`~/.zshrc` under your home directory, `Shine state/...` under the configured Shine state directory,
and `App data/...` under the platform application-data directory. Windows drive and network-share
paths retain their roots. These display labels do not broaden the listed permissions.
Confirmation asks `Apply this Plan?`; only Plans listing external/overlay code also ask you to
allow that code for this operation and explain that no persistent trust will be saved.

Shell command installations are grouped by command count, with Shell integration shown separately.
Internal state and recovery files share one path count, while backup associations with user files
remain explicit. Use `--verbose` for individual entries.

For App files and managed Sys items confirmed unchanged, required permissions omit their operation
effects. The default `upgrade` review hides scopes with no actions, required permissions, code
boundaries, blockers, or exceptional diagnostics; it also hides ordinary unchanged steps and
manual App generators skipped by routine upgrade. `upgrade --verbose` expands the remaining steps,
exact permissions, snapshot identities, diagnostics, and fingerprints while omitting those same
unchanged scopes and steps. Use `upgrade --verbose --full-plan` to see every planned scope and step;
`--full-plan` requires `--verbose`. State observations still bind approval, so a change after
review requires a new Plan. Active generators, hooks and shared transactions retain their own
permissions. Normal transaction codes and lifecycle snapshot identities appear only with
`--verbose`; preservation, blocking and other diagnostic notices remain explicit.

The default upgrade review groups routine App and Shell preset cache writes under internal
maintenance. These source copies do not mean application configurations or commands have changed.
Cache-only App work appears there rather than under `App Configs`, and missing-source preservation
notices appear under Warnings. Actual App changes, code execution, blockers, conflicts, and effects
that cannot be safely grouped remain explicit. Use `upgrade --verbose` to see the affected
categories, individual cache steps, and exact permissions. The complete Plan still binds approval.

After approval, `upgrade --verbose` also omits unchanged App files, already-installed managed Sys
items, and the total number of installed Shell categories. It still shows changed resources,
conflicts, warnings, and failures in detail. `upgrade --verbose --full-plan` includes the
unchanged execution rows. An ordinary fully unchanged run reports `Nothing to upgrade.`
Both targeted and whole-configuration upgrades count user-modified managed Sys files as kept.
If configuration items fail, the footer says `Upgrade incomplete` and includes their failure
count; the command still exits nonzero. Non-fatal hook warnings retain their existing behavior.

During Shell upgrade, the Plan compares the managed profile and configured shell startup files
before review. It shows changed profile files under `Shell integration (internal)` and omits
unchanged startup files and their permissions. `shell/profile` is an internal Plan identity, not a
Shell Preset category.

Recovery operations, blocked Plans, and permissions without reliable classification still show
concrete paths. Code trust, administrator access, and other capability notices remain visible.
Summaries only change the display: they grant no blanket access to `~/.shine` and are not a script
sandbox. The earlier `--dry-run` preview is unchanged.

## Shell presets

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine shell recover [--yes] [--verbose]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--yes] [--verbose]
```

- Preview installation or uninstall with `--dry-run`.
- `shell list` shows the active presets, including overlay additions and overridden descriptions.
- Installation and repair commands suggested by `shell info` quote literal target names for
  POSIX shells on Unix and PowerShell on Windows, including names containing spaces or quotes.
- `--replace-managed` may overwrite managed content changed after installation. Inspect the target
  with `shine info <TARGET> --diff` first.
- If Shine reports an interrupted operation, run `shine shell recover`. Changed files are preserved
  and may require manual review. The result counts Shell resource actions that needed rollback or
  cleanup. Cleanup after a committed operation preserves its installed content.

- Shell uninstall identifies retained launcher paths when ownership conflicts prevent removal.
  It has no `--force` override: inspect those files before deciding whether to move or remove them.
  A conflicting file may remain even after its installation receipt is cleared; the report says so.

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
  user-modified managed destination. The installed receipt must belong to that exact category and
  source file; `--force` cannot refresh a destination owned by another source.
  Its summary distinguishes `user-modified (kept)` from execution failures. Either leaves the
  refresh incomplete and returns a nonzero exit status; other selected files may still refresh.
- `app uninstall --force` may delete user-modified managed content. Always preview it with
  `--dry-run`.
- Protected App files show the receipt mismatch, followed by one retained-content summary.
  Force-preview guidance is grouped by category and explains file deletion, conditional backup
  restoration, or receipt-owned JSON-key removal. Use `--force --dry-run` or `--verbose` for
  recorded backup paths; unrelated JSON keys remain.
- Use `shine app recover` when an interrupted App operation blocks later changes.

See [Manage application configuration](../guides/app-presets.md).

Installed App resources remain managed when their Preset category or an individual source file
is removed, renamed, or absent after switching sources. `list` keeps the category visible;
`list`, `update`, and `upgrade` explain missing sources and suggest an uninstall preview.
Upgrade preserves these resources by default.

Use `shine app uninstall <OLD_CATEGORY> --dry-run`, then `shine app uninstall <OLD_CATEGORY>`
to uninstall using the original installation receipt even when the Preset is unavailable.
The top-level `shine uninstall app/<OLD_CATEGORY>` supports the same recovery. A bare category
also considers installed targets whose sources are missing; use `app/` or `shell/` when names conflict.
Uninstall selects that category's receipts, not another category sharing its destination.
For copy installs, unchanged managed content is removed and an existing recorded backup is
restored. JSON merge installs remove only receipt-owned keys and preserve unrelated keys.
User-modified managed content and its receipt are retained unless `--force` is explicit.
If the destination is missing, uninstall clears its receipt and leaves any recorded backup in
place. A dry-run preserves files, backups, and receipts; it reports user modifications and
recorded backup restoration separately. Missing source code cannot be used to run teardown.

App and Shell uninstall distinguish `Uninstall complete`, `Uninstall incomplete`, and
`Uninstall preview`. Actual uninstall returns a nonzero exit status if protected or conflicting
resources remain, or an operation fails, including partial completion. Review the per-resource
results: a nonzero exit status does not undo completed removals. A successfully generated dry-run
returns zero even when it shows protection; preview errors still return nonzero.
Default uninstall review omits known unchanged cache steps and empty permission sections;
`--verbose` retains the complete review. Once an incomplete result and its reasons have been
shown, the command exits nonzero without repeating a generic `Error` line. Other errors remain
visible.

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

If the Shine release check cannot finish, `update` prints a warning and still reports configuration
updates. When no configuration updates are found, it explicitly says the version check is incomplete
instead of claiming there is nothing to update. A release-check failure alone remains non-fatal.

`info <TARGET>` and `update <TARGET>` limit deferred-generator notices to the selected App category
or file. Shell queries do not suggest generator evaluation. Explicit `--run-generators` evaluation
still covers the selected App category, even for a single-file target. Trust or evaluation failures
remain nonzero; an incomplete evaluation does not produce a `No update available` conclusion.

`update --verbose` includes installed managed system configurations even when no update is pending.
`up-to-date (recorded)` compares the desired configuration with the installation record; it is not a
live system check. Missing Preset sources are shown explicitly. App summaries distinguish generators
that were not evaluated, require trust, or failed evaluation from user-modified files.
These generator-state counts refer to categories, or individual files with `--diff`, rather than
the number of generator executions.

If an App category or file entry is removed from the active Preset, `update --verbose` (also with
`--diff`) still reports its retained installed content and offers an uninstall preview. These checks
do not remove files or installation records, even when the entire category is gone.

`update` is read-only. `upgrade` displays the planned changes and asks for approval when there is
an action; a fully unchanged run reports `Nothing to upgrade.` without confirmation. Use `--yes`
only after reviewing the same scope. `--pull` first updates eligible Git-managed Preset sources.
`--prune-stale` permits removal of unchanged managed entries no longer present in the Preset.
`trust grant --development` keeps code edits from the displayed local source trusted while the
target, capability, source directory, and source layer remain unchanged. This is long-term source
authorization, not continuous code review.
`trust list` groups grants that share one security scope into a compact row and checks them against
the active Preset; `--verbose` expands capabilities, development-source labels, and review guidance.
Its review commands preserve the complete target as one argument, including spaces and shell
punctuation; use POSIX shells on Unix and PowerShell on Windows. Listing never renews a grant.
Concurrent `trust grant` and `trust revoke` commands preserve changes to other targets; a completed
revocation cannot be restored by another command saving an older copy of the trust store.

Interactive lifecycle confirmation may authorize external code for that operation without saving
a grant. Automation and `--yes` require existing trust; Shell live requires Development trust.

Missing Presets, user-modified files, foreign command entries, missing env/admin contracts, and untrusted
external code are reported as attention items rather than silently overwritten. Restore the source
or follow the command shown by Shine.

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

Managed configuration previews (`sys apply --dry-run` and `sys uninstall --dry-run`) mark the
section with `(dry-run)` and show `→ would update` or `→ would remove` for planned changes.
These rows do not indicate an applied change. Successful uninstall shows `✓ removed`.

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

Use `validate`, `lint`, `plan`, and `test` before distributing a Preset. These commands inspect
authoring input without installing it. `migrate --dry-run` previews legacy metadata changes;
applying a migration requires review and confirmation. A Git-managed overlay is a disposable mirror,
so edit its upstream checkout rather than the mirror.
If `preset schema` cannot write its output, it reports an error and exits nonzero.
Artifact follow-up commands printed by migration quote category names for POSIX shells on Unix
and PowerShell on Windows. Migration itself does not run those commands.
New templates omit empty and speculative capability tables. `--unrestricted` adds the optional
unrestricted author statement, but all arbitrary code is unisolated with or without it.

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
shine env workspace export --format files [--workspace <FILE>] --mode <MODE> --file-group <NAME>... [--dry-run]
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
Implicit writes reject shadowed keys. Explicit targets may maintain shadowed entries and report
both the saved file and current effective source. The old `--force` for `set`, `delete`, and
`secret encrypt` is rejected without writing; use explicit scope and file options instead.

`env explain --reveal`, `env list --reveal`, `env get`, `env secret decrypt`, exports with `--include-secrets`, and child
commands started by `env run` can expose plaintext. Run them only in a trusted terminal and never
put secrets directly in command arguments or documentation.

Broker policy commands are documented with the complete remote workflow in
[SSH sessions, secret brokering, and file transfer](../guides/ssh-transfer.md). For local values,
workspace encryption, command wrappers, and hybrid backends, see
[Manage environment variables and secrets](../guides/environment.md).

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

Shine checks for manual maintenance on ordinary interactive invocations. Checks use one of four
schedules: every invocation, once per minute, once per hour, or once per day. A due check runs when
Shine is invoked; there is no permanent background service.

The retired HTTP service's registration files are checked on each eligible invocation. Native
service status is checked at most hourly by a short-lived background process, and its result is
shown on a later invocation. The same maintenance action is normally reminded at most once per day.
Checks only inspect state; they do not remove services, delete files, or execute Presets.

Run all current maintenance checks immediately, including when configuration cannot be loaded:

```text
shine state check
```

Maintenance reminders and incomplete-check warnings use bold yellow when stderr supports color,
with one blank line separating the reminder block from command output. `NO_COLOR` disables color.

Automatic reminders go to terminal stderr. They are skipped in noninteractive use and for help,
completion, Preset authoring, environment commands, command runners, SSH/transfer, theme sync,
binary management, migration, cleanup, and recovery commands. Explicit `state check` also works
with redirected output and repeats findings immediately. An incomplete check is reported there;
automatic check failures do not block ordinary commands.

If another check or cleanup runs in the same configuration directory while `state check` is
inspecting, the older check asks you to retry and exits with a nonzero status instead of reporting
completion.

The existing version check and required patch-upgrade policy are unchanged. Maintenance checks
do not replace permission, trust, compatibility, or approval checks before an operation.

### Remove the retired HTTP service

```text
shine clear serve
```

`clear` requires an explicit target; running it alone does not clean up everything. `clear serve`
stops and removes an older version's user service without loading Shine configuration or deleting
published files. Repeating successful cleanup is safe; service-manager errors are reported.
Successful cleanup resets its cached reminders and discards in-flight results in the selected
configuration directory. Use the same `--config-dir` or `SHINE_CONFIG_DIR` when checking that state.
The `shine serve` namespace is removed. See
[Installation and upgrades](../installation.md#retire-an-older-local-http-service).

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
```

`stable` follows released versions. `preview` follows the replaceable preview build. See
[Installation and upgrades](../installation.md).

See [temporary secret files](../guides/environment.md#workspace-secret-files) for file-group configuration, sealing, persistent export, execution, and cleanup boundaries.
