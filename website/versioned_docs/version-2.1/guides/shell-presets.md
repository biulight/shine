---
title: Manage shell presets
sidebar_position: 1
---

# Manage shell presets

Shell presets install scripts into Shine's managed directory and create directly callable command
entries in `~/.shine/bin/`. Shine manages profiles and command directories for Bash, Zsh, and
PowerShell. Native entries use `.sh` or `.ps1`; Bun is also available as a cross-platform command
runtime.

See [built-in presets](../reference/built-in-presets.md#shell-presets) for categories, platform
limits, commands available in the current session, and required environment variables.

## Browse and install

```bash
shine shell list
shine shell install proxy
shine shell install utils/shine-env-export # Install only this command
shine shell install            # Install every category available on this platform
```

You can also let Shine identify whether a category is a shell or application preset:

```bash
shine install proxy
shine install shell/utils/shine-env-export
```

A category target activates every command available for the current platform. Use an explicit
`category/command` target when you need only one command. Mutation does not accept a bare command
name because the same name may appear in more than one category.

Open a new terminal or reload the shell profile after installation. To install completions, run:

```bash
shine completions install
```

## Repair an installation

Rebuild managed scripts, command entries, and the `PATH` fragment from the active preset:

```bash
shine shell install proxy --replace-managed
shine install shell/proxy --replace-managed
```

`--replace-managed` overwrites the corresponding Shine-managed content. Inspect
`shine info shell/proxy --diff` first so that intentional local changes are not mistaken for damage.

## If installation, upgrade, or uninstall is interrupted {#recover-an-interrupted-shell-transaction}

If a Shell preset operation is interrupted, Shine may pause later changes and ask you to
run recovery:

```bash
shine shell recover
```

Review the recovery plan and confirm to proceed. Depending on where the operation stopped,
Shine rolls back unfinished changes or keeps completed changes and cleans up leftover files.

If relevant files have changed since the interruption, Shine stops recovery and preserves them
to avoid overwriting your edits. Keep the error details and affected files, and follow the
reported guidance to investigate. Do not manually delete recovery records or rollback files.

For automation, use `shine shell recover --yes` to skip interactive confirmation. Safety checks
still apply.

## Uninstall

```bash
shine shell uninstall proxy --dry-run
shine shell uninstall proxy
shine shell uninstall utils/shine-env-export
shine shell uninstall proxy --purge
```

Non-dry-run install and uninstall, plus `shine upgrade`, show a snapshot-bound lifecycle Plan. The
confirmation defaults to No; automation must pass `--yes`, which still renders and revalidates all
steps and permissions. `--dry-run` remains a separate preview and cannot be combined with `--yes`.

Command-scoped uninstall preserves other installed commands in the category. Shared preset or
snapshot files may remain while a sibling command still needs them. `--purge` also removes empty
managed preset directories; without a target, uninstall processes the whole shell preset tree. It
never removes `~/.shine/config.toml`.

## Common built-in commands

| Category | Commands | Purpose |
| --- | --- | --- |
| `image-tools` | `img-compress`, `img-resize`, `img-convert` | Batch-process JPEG, PNG, and WebP files with Bun 1.3.14 or newer |
| `proxy` | `setproxy`, `usetproxy` | Set or clear proxy variables in the current terminal session |
| `utils` | `copyfile` | Copy file content to the local clipboard through OSC 52 |
| `utils` | `shine-env-export` | Load a Shine environment value into the current shell |
| `utils` | `shine-theme-sync` | Print shell `export` statements for the terminal light/dark theme |
| `agent` | `ccenv` | Select a Codex, DeepSeek, or Qwen provider and launch Claude Code in an isolated child environment; requires Bun |

Some categories provide different scripts by platform. `shine shell list` shows only entries
available on the current platform.

By default, `ccenv` connects to CLIProxyAPI at `http://127.0.0.1:8317` for Codex and can interactively
select DeepSeek or Qwen. Credentials use `CLIPROXYAPI_AUTH_TOKEN`, `DEEPSEEK_API_KEY`, or
`QWEN_API_KEY`. Encrypted values use the corresponding `_SECRET` suffix; legacy `_GPG_SECRET`
values remain readable. Provider variables are passed only to the launched Claude process and never
modify the current terminal. Claude arguments are forwarded unchanged. If the first argument
conflicts with a `ccenv --run` compatibility argument, insert `--` first:

```bash
ccenv --print "hello"
ccenv -- --run
```

To write a cross-platform command preset with Bun, see
[Shell entries with an optional runtime](./custom-presets.md#shell-entries-with-an-optional-runtime).
