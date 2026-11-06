---
title: Tasks and application artifacts
sidebar_position: 6
---

# Tasks and application artifacts

This guide shows how to save personal command shortcuts and run application artifact scripts.

## Save common commands

Save a complete command as a task:

```bash
shine task save logs -- tail -f /var/log/example.log
shine task save kill-port -- sh -c 'lsof -ti :3000 | xargs kill'
shine task save project-check --cwd ~/src/example -- bun run check
```

Run it later from any directory:

```bash
shine task run logs
shine run logs
shine task list
shine task info kill-port
```

Tasks are saved in `tasks.toml` under the active Shine configuration directory. Use
`SHINE_CONFIG_DIR` or `--config-dir` to select a different task list. A task saved with `--cwd`
always starts in that directory; without it, the task uses your current directory. `--cwd` accepts
`~` but requires a UTF-8 path.

If saving or deleting a task times out because another command is updating the list, wait for that
command to finish and retry. Do not delete `tasks.lock` to force an update.

Tasks do not automatically invoke a shell. For pipes, redirection, variable expansion, or globs,
include a shell such as `sh -c '...'`. Windows does not include `sh`; choose an installed shell.
Task output can be piped to other commands, and a failed task returns the program's exit code.

Replace or delete a task:

```bash
shine task save logs --force -- journalctl -f
shine task delete logs
```

Task names may contain letters, numbers, dots, hyphens, and underscores and must start with a letter
or number.

## Generate application helper resources

Some application presets declare an artifact script that generates extra files from current
configuration:

```bash
shine app artifact apply surge
```

`app artifact apply` is explicit. `shine app install`, `shine update`, and `shine upgrade` do not
implicitly run artifact scripts. It displays a security Plan; non-interactive callers must add
`--yes`. A script failure fails the command and its output is shown directly.

Artifact scripts receive configured `[artifact].env` sources only, plus Shine-provided path
variables:

| Variable | Purpose |
| --- | --- |
| `SHINE_APP_ID` | Current application preset ID |
| `SHINE_APP_DIR` | Temporary App snapshot for this invocation, containing effective base and overlay files |
| `SHINE_APP_SOURCE_DIR` | Same effective source snapshot directory as `SHINE_APP_DIR` |
| `SHINE_APP_OVERLAY_DIR` | Captured effective overlay files for this invocation; explicitly empty without an overlay, overriding inherited values |
| `SHINE_CONFIG_DIR` | Current Shine runtime directory |
| `SHINE_CACHE_DIR` | Current application cache directory |
| `SHINE_STATE_DIR` | Current application state directory |

Each structured `env` entry declares its source, optional target alias, and sensitivity together; missing optional sources
are omitted. Values are passed as stored. Secret-classified inputs are not decrypted automatically and do not trigger GPG, age, or Touch ID prompts.

Generators, hooks, and artifacts run from a temporary copy of the reviewed preset. Write files you
want to keep to `SHINE_STATE_DIR` or `SHINE_CACHE_DIR`; the temporary copy is removed afterward.
Scripts run with your permissions and are not sandboxed.
