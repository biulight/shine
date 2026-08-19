---
title: Command reference
sidebar_position: 1
---

# Command reference

This page reflects Shine 1.4.0. Use `--help` on any subcommand for the exact interface of the
installed version.

## 1.0 target rules

Canonical targets are `app/<category>`, `shell/<category>`, and `sys/<item>`. Installation and
uninstall also accept a bare category when it is unique across app and shell presets. Prefer complete
targets in scripts and documentation to avoid future ambiguity.

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

Since 1.0, `install --replace-managed` replaces `reinstall`. Legacy top-level `clear`, `pull`,
`export`, `link`, and `overlay`, plus `app build/unbuild`, `sys init`, and `env show`, have no
compatibility aliases.

## Top-level commands

| Command | Purpose |
| --- | --- |
| `shine init [--yes]` | Create project `shine.config.toml` in the current directory |
| `shine shell <SUBCOMMAND>` | Manage shell command presets |
| `shine app <SUBCOMMAND>` | Manage application configuration presets |
| `shine install <TARGET> [--replace-managed]` | Install or repair an app/shell target |
| `shine uninstall <TARGET> [--force] [--purge] [--dry-run]` | Uninstall an app/shell target |
| `shine completions <SUBCOMMAND>` | Generate or install shell completions |
| `shine list [--available [KIND]]` | List installed resources, or browse available `app`, `shell`, and `sys` catalogs |
| `shine info <TARGET> [--diff] [--verbose]` | Inspect an available or installed app/shell target or `sys/<ITEM>` |
| `shine update [TARGET]` | Check managed content and stable Shine updates |
| `shine upgrade [TARGET]` | Apply all or selected app, shell, and managed-system updates |
| `shine preset <SUBCOMMAND>` | Manage sources, overlays, exports, and Git synchronization |
| `shine state migrate [--dry-run]` | Migrate and clean legacy runtime state |
| `shine self <SUBCOMMAND>` | Install or upgrade the Shine binary |
| `shine serve <SUBCOMMAND>` | Publish resources under `~/.shine/http/` through a local HTTP service |
| `shine env <SUBCOMMAND>` | Manage preset variables, workspace environments, proxies, and secrets |
| `shine sys <SUBCOMMAND>` | Manage system bootstrap and managed system configuration |
| `shine theme sync` | Detect light/dark terminal appearance and print shell exports |
| `shine ssh ...` / `shine local ...` | Open SSH, broker secrets, and transfer files with POSIX remotes |
| `shine task <SUBCOMMAND>` / `shine run <NAME>` | Save and run personal commands |

Every command accepts global `--config-dir <PATH>` to select the global configuration and runtime
directory temporarily.

## Shell and application presets

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [CATEGORY] [--replace-managed]
shine shell uninstall [CATEGORY] [--purge] [--dry-run]

shine app list
shine app info <CATEGORY>
shine app install [CATEGORY] [--dry-run] [--replace-managed]
shine app refresh <CATEGORY> [FILE] [--force]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run]
shine app artifact apply <APP_ID>
shine app artifact remove <APP_ID>
```

`--replace-managed` overwrites managed content modified after installation; inspect
`shine info <TARGET> --diff` first. `app uninstall --force` deletes user-modified managed files, so
preview with `--dry-run`.

`app refresh` handles only generated files tracked by the manifest and preserves the last successful
content on failure. Artifact apply/remove explicitly runs an external integration declared by the
preset; ordinary installation and upgrade do not implicitly apply it.

## Status, updates, and completions

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale]
shine state migrate [--dry-run]
shine completions install
shine completions <bash|zsh|powershell>
```

- `update --refresh-release` bypasses the 24-hour cache. By default, `update` groups targets under
  the same Homebrew-style sections as `shine list`: interactive terminals use horizontal columns,
  while redirected output stays one target per line. It then prints the `shine upgrade` action once.
  App files collapse to their category. `update --diff` switches to detailed vertical rows, expands
  affected files, and shows available content changes.
- A targeted update cannot combine with `--verbose` or `--refresh-release`.
- `update/upgrade --pull` synchronizes Git-managed sources and reloads configuration first.
- `upgrade --prune-stale` removes old managed app files no longer present in the source.
- By default, `upgrade` prints each app category, Shell target, or managed-system item it actually
  updates and counts each user-facing target once. App rows include the number of changed files.
  `--verbose` expands app files and successful hook output, and also shows current/skipped items and
  Shell deployment details such as snapshots, templates, and Bin Links. Failures, conflicts,
  user-modified warnings, and blocked hooks remain visible without `--verbose`.
- `shell info` and top-level `info` inspect uninstalled presets; `list --available` filters by kind.

## System presets

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys update [ITEM] [--verbose] [--proxy]
shine sys bootstrap [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy]
shine sys apply [ITEM] [--dry-run]
shine sys uninstall <ITEM> [--dry-run]
```

`sys bootstrap` installs software and shell integration. `sys update` only checks recorded bootstrap
software and never upgrades it. `shine upgrade sys/<ITEM>` converges an independent managed item.

## Preset sources and customization

```text
shine preset new <app|shell> [--force]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset copy` copies one complete built-in category for a partial overlay; `preset export` exports the
full collection. External shell presets use snapshots by default and require `shine upgrade` after
source changes. `--live` is for preset development. See [Customize presets](../guides/custom-presets.md).

## Environment values and secrets

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env broker describe [--workspace <FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy <add|update> --name <NAME> --ssh-target <TARGET> [--project <PROJECT>] --workspace <FILE> [--remote-workspace <REMOTE_FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy diff <NAME> --workspace <FILE> --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy list
shine env broker policy info <NAME>
shine env broker policy remove <NAME>
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity list
```

`--with` is repeatable and accepts `KEY=ALIAS`. `--no-workspace` uses explicit values and the process
environment only and conflicts with `--workspace` and `--mode`. Workspace initialization currently
requires `--from-dotenv` and supports `--dry-run`. Broker policy creation chooses one or more explicit
`--release` keys or freezes every currently declared key with `--release-all-declared`; the forms are
mutually exclusive. Touch ID identities are macOS-only and require `age-plugin-se`.

For broker policies, `--project` stores a human-readable project label. `--remote-workspace`
requires remote requests to report that exact absolute workspace path in addition to matching the
workspace contents and other policy fields.

An environment proxy creates a same-name shim under `~/.shine/bin/` and injects only declared values
into its child, preferring `<KEY>_SECRET` over `<KEY>`. Disable retains the shim without injection.
Project rules require a discoverable `shine.config.toml` and override same-name global rules.
Uninstall removes the managed shim and user-level rule.

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

Tasks store and execute argument arrays without a shell. `--cwd` fixes the working directory; without
it the caller's directory is used. `serve install` currently supports a macOS user service only;
`start` runs the local service in the foreground.

## SSH, secret brokering, and transfer

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [--secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session]] [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine options must precede the SSH target. Remote on-demand requests use
`shine env run --secret-broker`; see the [SSH guide](../guides/ssh-transfer.md#provide-secrets-to-remote-commands-on-demand).
A Windows remote uses `--remote-shell windows` for PowerShell injection only, without transfer or
Secret Broker support.

## Program installation and upgrades

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

Stable `shine --version` output is `shine 1.4.0 (<commit> <date>)`; preview builds use a label such as
`1.4.0-preview`.
