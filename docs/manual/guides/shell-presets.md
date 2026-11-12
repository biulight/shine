---
title: Manage shell presets
sidebar_position: 1
---

# Manage shell presets

Shell presets install reusable scripts as commands you can call directly in Bash, Zsh, or
PowerShell. Some commands need an additional runtime such as Bun; check the preset's requirements
before installing.

See [built-in presets](../reference/built-in-presets.md#shell-presets) for categories, platform
limits, commands available in the current session, and required environment variables.

## Browse and install

List the presets first, then choose one installation scope:

| Operation | Command |
| --- | --- |
| List Shell presets | `shine shell list` |
| Install the proxy category | `shine shell install proxy` |
| Install only this command | `shine shell install utils/shine-env-export` |
| Install every category available on this platform | `shine shell install` |

Installing a category enables all its commands available on the current platform. Use
`category/command` to select just one. Do not pass only a command name, since commands in different categories
can share a name.

Before installing, review the commands, configuration changes, backups, and permissions in the
plan. Confirmation defaults to No. Add `--verbose` for detailed paths and permissions.

Shell commands run with your permissions without sandbox isolation. Review and authorize external
or overlay code before using it. Interactive confirmation can approve one operation; automation
with `--yes` requires an existing trust grant. This option skips only the prompt, keeps safety
checks, and cannot be combined with `--dry-run`. See
[Custom presets](./custom-presets.md#declare-permissions) for authorization steps.

Open a new terminal or reload the shell profile after installation. To install completions, run:

```bash
shine completions install
```

## Repair an installation

If command entries or shell configuration are damaged, inspect differences before replacing
managed content with the preset:

```bash
shine info shell/proxy --diff
shine shell install proxy --replace-managed
```

`--replace-managed` overwrites managed content, including your local edits.

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
When restoring shell configuration, Shine preserves unrelated content.

If relevant files have changed since the interruption, Shine stops recovery and preserves them
to avoid overwriting your edits. Keep the error details and affected files, and follow the
reported guidance to investigate. Do not manually delete recovery records or rollback files.

For automation, use `shine shell recover --yes` to skip interactive confirmation. Safety checks
still apply.

## Uninstall

| Operation | Command |
| --- | --- |
| Preview category removal | `shine shell uninstall proxy --dry-run` |
| Apply after review | `shine shell uninstall proxy` |
| Alternative: remove only this command | `shine shell uninstall utils/shine-env-export` |

Review and confirm the uninstall plan. Removing one command preserves other installed commands
in its category and the shared files they need. External preset sources, `~/.shine/config.toml`,
and unrelated shell configuration are preserved.

Without a target, `shine shell uninstall` removes all Shell presets, remaining managed startup
configuration, and the entire internal Shell cache, including extra files you stored there.
Targeted category or command removal preserves those extra cache files. Check the scope with
`shine shell uninstall --dry-run` first.

Uninstall and recovery do not require trust in the code being removed. If an old script still uses
`--purge`, remove it as described in [Upgrades and migration](./upgrading.md#upgrade-to-shine-26).

## Update external presets

In the default snapshot mode, source edits affect installed commands only after you review and
apply an update. Use `shine update shell/<CATEGORY>` to inspect changes, then
`shine upgrade shell/<CATEGORY>`. Shared-file changes can affect other installed commands in the
category; review the entire scope in the plan. Commands not already installed are not added.

Live mode uses edited source on the next invocation and requires `--development` trust.
Use it only when you trust future code changes from that source; ordinary snapshot grants do not
permit live mode. If `list` or `info` reports `development trust required`, review the source and
follow the authorization guidance. See [Custom presets](./custom-presets.md) for setup.

## Common built-in commands

| Category | Commands | Purpose |
| --- | --- | --- |
| `image-tools` | `img-compress`, `img-resize`, `img-convert` | Batch-process JPEG, PNG, and WebP files with Bun 1.3.14 or newer |
| `proxy` | `setproxy`, `usetproxy` | Set or clear the session proxy and Yarn's persistent proxy settings |
| `utils` | `copyfile` | Copy file content to the local clipboard through OSC 52 |
| `utils` | `shine-env-export` | Load a Shine environment value into the current shell |
| `utils` | `shine-theme-sync` | Apply terminal light/dark theme settings to the current session |
| `agent` | `ccenv` | Select a Codex, DeepSeek, or Qwen provider and launch Claude Code with provider-specific environment values; requires Bun |

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
