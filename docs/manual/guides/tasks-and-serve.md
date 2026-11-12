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
always starts in that directory; without it, the task uses your current directory. `--cwd` accepts `~`.

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

Shine shows a plan and asks for confirmation before running the script. Scripts run with your
permissions without sandbox isolation; review external code before approving it. Non-interactive
runs require `--yes` and an existing trust grant for external code. A script failure fails the
command, with details shown in the terminal.

Some presets also declare automatic actions after installation or upgrade; those actions require
review and approval too. `shine update` only checks for updates. Follow the
[application preset guide](./app-presets.md) for setup, prerequisites, and removal instructions.

If you want to write an artifact script, see [Custom presets](./custom-presets.md) for its runtime,
input declarations, and output directories.
