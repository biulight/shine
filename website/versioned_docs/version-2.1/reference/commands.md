---
title: Command reference
sidebar_position: 1
---

# Command reference

This page reflects Shine 2.1.0. Run `shine <COMMAND> --help` for every option supported by the
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
| `shine install <TARGET>` | Install or repair an app or shell target |
| `shine uninstall <TARGET>` | Uninstall an app or shell target |
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

## Shell presets

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes]
shine shell recover [--yes]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes]
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
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes]
shine app refresh <CATEGORY> [FILE] [--force] [--yes]
shine app recover [--yes]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes]
shine app artifact apply <APP_ID> [--yes]
shine app artifact remove <APP_ID> [--yes]
```

- `app info` and `update` do not run generators unless `--run-generators` is present.
- `app refresh` runs a generated-file refresh explicitly. `--force` permits replacing a
  user-modified managed destination.
- `app uninstall --force` may delete user-modified managed content. Always preview it with
  `--dry-run`.
- Use `shine app recover` when an interrupted App operation blocks later changes.

See [Manage application configuration](../guides/app-presets.md).

## Status, updates, trust, and completions

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <app/CATEGORY|sys/ITEM>
shine trust grant <app/CATEGORY|sys/ITEM> [--yes]
shine trust list
shine trust revoke <app/CATEGORY|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

`update` is read-only. `upgrade` displays the planned changes and asks for approval; use `--yes`
only after reviewing the same scope. `--pull` first updates eligible Git-managed Preset sources.
`--prune-stale` permits removal of unchanged managed entries no longer present in the Preset.

Missing Presets, user-modified files, foreign command entries, missing permissions, and untrusted
external code are reported as attention items rather than silently overwritten. Restore the source
or follow the command shown by Shine.

## System presets

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys recover [--yes]
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes]
shine sys profile enable <ITEM> [--dry-run] [--yes]
shine sys profile disable <ITEM> [--dry-run] [--yes]
shine sys apply [ITEM] [--dry-run] [--yes]
shine sys uninstall <ITEM> [--dry-run] [--yes]
```

Use `bootstrap` to ensure selected software and shell integration are present. Use `apply` and
`uninstall` for reversible managed system configuration. These operations may request administrator
access after you approve their plan. `--force-profile` may replace conflicting profile content, so
review a dry run first.

See [Initialize and manage a system](../guides/system-init.md).

## Preset authoring and sources

```text
shine preset new <app|shell|sys> [--force]
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
