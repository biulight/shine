---
title: Command reference
sidebar_position: 1
---

# Command reference

This page reflects Shine 2.3.0. Run `shine <COMMAND> --help` for every option supported by the
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
| `shine serve ...` | Serve resources under `~/.shine/http/` |
| `shine self ...` | Install or upgrade the Shine binary |
| `shine state migrate` | Preview or apply supported legacy-state migrations |
| `shine trust ...` | Review target-scoped trust for external Preset code |
| `shine completions ...` | Generate or install shell completions |
| `shine theme sync` | Print terminal-theme environment exports |

Every command accepts the global `--config-dir <PATH>` option.

Security Plans show user file paths explicitly and summarize installed files, Shine maintenance,
and associated backup/recovery files by purpose, with access types and path counts. Installation,
upgrade, uninstall, recovery, App refresh/artifact, and Sys bootstrap/profile/apply reviews accept
`--verbose` to expand file paths, steps, snapshot identities, and diagnostics in the displayed
review scopes.

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

The default upgrade review combines routine App and Shell preset cache writes into one internal
maintenance summary. These source copies are not application configuration or command updates.
Use `upgrade --verbose` for the affected categories and individual cache steps. Cache conflicts,
preservation warnings (including missing old sources), code boundaries, and required permissions
remain explicit; the complete Plan still binds approval. When an App scope contains only cache
maintenance and missing-source preservation notices, its Core-attributed cache permissions appear
under internal maintenance and its preservation notices appear under Warnings. It does not create
an App Configs section. Actual App changes, code, blockers, or ambiguous permission attribution
retain the App review section.

After approval, `upgrade --verbose` also omits unchanged App files, already-installed managed Sys
items, and the total number of installed Shell categories. It still shows changed resources,
conflicts, warnings, and failures in detail. `upgrade --verbose --full-plan` includes the
unchanged execution rows. An ordinary fully unchanged run reports `Nothing to upgrade.`

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
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes] [--verbose]
```

- Preview installation or uninstall with `--dry-run`.
- `--replace-managed` may overwrite managed content changed after installation. Inspect the target
  with `shine info <TARGET> --diff` first.
- If Shine reports an interrupted operation, run `shine shell recover`. Changed files are preserved
  and may require manual review.

See [Manage shell presets](../guides/shell-presets.md).

## Application presets

```text
shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine app refresh <CATEGORY> [FILE] [--force] [--yes] [--verbose]
shine app recover [--yes] [--verbose]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes] [--verbose]
shine app artifact apply <APP_ID> [--yes] [--verbose]
shine app artifact remove <APP_ID> [--yes] [--verbose]
```

- `app info` and `update` do not run generators unless `--run-generators` is present.
- `app refresh` runs a generated-file refresh explicitly. `--force` permits replacing a
  user-modified managed destination. The installed receipt must belong to that exact category and
  source file; `--force` cannot refresh a destination owned by another source.
- `app uninstall --force` may delete user-modified managed content. Always preview it with
  `--dry-run`.
- Use `shine app recover` when an interrupted App operation blocks later changes.

See [Manage application configuration](../guides/app-presets.md).

## Status, updates, trust, and completions

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--full-plan] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine trust grant <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM> [--development] [--yes]
shine trust list [--verbose]
shine trust revoke <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

`update` is read-only. `upgrade` displays the planned changes and asks for approval when there is
an action; a fully unchanged run reports `Nothing to upgrade.` without confirmation. Use `--yes`
only after reviewing the same scope. `--pull` first updates eligible Git-managed Preset sources.
`--prune-stale` permits removal of unchanged managed entries no longer present in the Preset.
`trust grant --development` keeps code edits from the displayed local source trusted while the
target, capability, source directory, and source layer remain unchanged. This is long-term source
authorization, not continuous code review.
`trust list` groups grants that share one security scope into a compact row and checks them against
the active Preset; `--verbose` expands capabilities, development-source labels, and review guidance.

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

Use `bootstrap` to ensure selected software and shell integration are present. Use `apply` and
`uninstall` for reversible managed system configuration. These operations may request administrator
access after you approve their plan. `--force-profile` may replace conflicting profile content, so
review a dry run first.

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
New templates omit empty and speculative capability tables. `--unrestricted` adds the optional
unrestricted author statement, but all arbitrary code is unisolated with or without it.

See [Customize presets](../guides/custom-presets.md) and
[Migrate system presets to v2](../guides/sys-preset-v2-migration.md).

## Environment values and secrets

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity init --phone [--recipient-type <tag|phone>] [--label <LABEL>] [--transport <auto|adb|qr>] [--adb-serial <SERIAL>]
shine env secret identity list
```

`env list --reveal`, `env get`, `env secret decrypt`, exports with `--include-secrets`, and child
commands started by `env run` can expose plaintext. Run them only in a trusted terminal and never
put secrets directly in command arguments or documentation.

Broker policy commands are documented with the complete remote workflow in
[SSH sessions, secret brokering, and file transfer](../guides/ssh-transfer.md). For local values,
workspace encryption, command wrappers, and hybrid backends, see
[Manage environment variables and secrets](../guides/environment.md).

## Tasks, local service, and theme

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine serve install [--port <PORT>]
shine serve start [--port <PORT>]
shine serve status
shine serve uninstall
shine serve url <PATH> [--port <PORT>]

shine theme sync [--auto] [--quiet]
```

Saved tasks execute the stored command, optionally from its stored working directory. The local
service publishes files under `~/.shine/http/`; do not place secrets there.

See [Tasks and the local service](../guides/tasks-and-serve.md) and
[Synchronize the terminal theme](../guides/terminal-theme-sync.md).

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
