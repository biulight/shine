---
title: Quick start
sidebar_position: 3
---

# Quick start

This walkthrough uses the proxy shell preset to complete one browse, install, use, inspect, and
uninstall-preview cycle.

## 1. Browse available presets

```bash
shine shell list
shine app list
shine sys list
shine list --available
```

The first three commands list shell commands, application configuration, and initialization items
for the current operating system. `list --available` uses the unified 1.0 catalog for all three
resource types; append `app`, `shell`, or `sys` to filter it.

## 2. Install the proxy commands

```bash
shine shell install proxy
# Equivalent canonical target: shine install shell/proxy
```

Shine places the scripts under `~/.shine/presets/shell/`, creates command entries in
`~/.shine/bin/`, and adds that directory to supported shell profiles.

Open a new terminal or reload the current shell configuration:

```bash
source ~/.zshrc
# For bash: source ~/.bashrc
```

PowerShell users can reopen the terminal to load the updated profile.

## 3. Use and inspect the preset

```bash
setproxy
shine list
shine info shell/proxy
```

`shine list` reports the installed lifecycle unit, `proxy`; `shine info shell/proxy` expands it into
the managed commands, links, and source details. Use the full target `shell/proxy` in scripts. A bare
name is accepted only when it is unique across application and shell categories.

Disable the proxy in the current terminal session:

```bash
usetproxy
```

## 4. Preview a safe uninstall

```bash
shine shell uninstall proxy --dry-run
```

After reviewing the output, remove `--dry-run` to apply the uninstall. Shine removes only the
scripts, command entries, and profile fragments it manages.

Next, choose the workflow that matters to you:

- install [application configuration](./guides/app-presets.md), including the opinionated Surge and
  Clash Verge Rev artifact workflows;
- initialize a machine with the deliberately scoped [system bootstrap](./guides/system-init.md);
- save [repeatable tasks](./guides/tasks-and-serve.md); or
- continue work through [SSH environment forwarding, file transfer, and the local secret
  broker](./guides/ssh-transfer.md).
