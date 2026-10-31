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

A normal built-in Shell installation shows a short summary before confirmation:

```text
Install · utils
  Source       built-in · Shine distribution
  Commands     copyfile, shine-env-export, shine-theme-sync
  Changes      install 3 commands; update shell integration and ~/.bashrc
  Backup       ~/.bashrc

Apply? [y/N]
```

The commands, changed profile paths, and recovery backup paths reflect the current Plan.
Use `--verbose` for cache and state files, exact permissions, author statements, and identities.
External code, code executed during installation, additional permissions, conflicts, and unfamiliar
diagnostics retain the expanded review.

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

If a command conflicts with an existing file, inspect the reported path before removing or
replacing it. After switching preset sources or updating Shine, run
`shine upgrade shell/<CATEGORY>` to refresh installed commands. External code still requires review
and authorization.

## If installation, upgrade, or uninstall is interrupted {#recover-an-interrupted-shell-transaction}

If another Shell operation is still running, wait for it to finish. If Shine reports that the
reviewed changes are no longer current, review the new Plan before retrying.

If a Shell preset operation is interrupted, Shine may pause later changes and ask you to
run recovery:

```bash
shine shell recover
```

Review the recovery plan and confirm to proceed. Depending on where the operation stopped,
Shine rolls back unfinished changes or keeps completed changes and cleans up leftover files.
When restoring a managed block in your shell profile, Shine preserves unrelated content and the
current file permissions on Unix.

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
```

Uninstall and `shine upgrade` also show their planned changes.
Confirmation defaults to No; automation must pass `--yes`, while the Plan and safety checks still
run. `--dry-run` remains a separate preview and cannot be combined with `--yes`.

Shell commands run with your permissions and are not sandboxed. External and overlay commands
require confirmation of the current Plan or a matching trust grant, regardless of their permission
declarations. Follow the reported `shine trust inspect` and `shine trust grant` commands to review
blocked operations. Only use `--development` when you also trust future edits from that source.
See [custom Preset capabilities](./custom-presets.md#declare-permissions).

Snapshot trust works with the default snapshot deployment. It copies the reviewed category content
into Shine's managed directory; later source edits do not enter the installed command until an
explicit reviewed update. `external_shell_mode = "live"` requires development trust because later
invocations or `source` operations read from a mutable trusted source. Snapshot trust cannot be used with live deployment.

Updating shared files can affect other installed commands in the same category. The Plan lists
all affected commands for review. Each affected external command needs one-time human consent or a matching grant for the new snapshot;
uninstalled siblings do not need a grant and are not installed. Existing live launchers are kept
after upgrading Shine, but `list`/`info` reports `development trust required` until they are reviewed.
Uninstall and recovery remain available without granting trust to code being removed.

Use `shine list` to inspect installed resources; `shine shell list` shows available Shell presets.
Command-scoped uninstall preserves other installed commands in the category. Shared preset or
snapshot files may remain while a sibling command still needs them.
After uninstalling the last category, `shine shell uninstall` can remove the remaining managed
profile and startup integration. Other content in your shell configuration is preserved; preview
the cleanup with `shine shell uninstall --dry-run` before approving it.

Uninstall includes empty managed directory cleanup. Without a target, it removes the entire internal
Shell preset cache tree, including extra files stored there. With a category or command target,
extra cache files are preserved, as are shared files still needed by installed sibling commands.
External preset source directories are always preserved; managed launchers and
installation snapshots are still removed as applicable. Uninstall never removes
`~/.shine/config.toml`. Review the Plan and use `--dry-run` to inspect the selected scope.

`shine uninstall shell/<category>[/<command>]` uses the same target-limited behavior.
`--purge` has been removed; omit it from existing commands. Unlike targeted Shell uninstall,
App category uninstall removes its entire internal category cache, including extra files;
see [App presets](app-presets.md).

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
