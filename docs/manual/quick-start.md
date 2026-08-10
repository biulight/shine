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

`shine list` shows installed content that is currently usable. `shine info` can also inspect presets
that are not installed. Use a full target such as `shell/proxy` in scripts. A bare name is accepted
only when it is unique across application and shell categories.

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

Next, install [application configuration](./guides/app-presets.md) or use
[system initialization presets](./guides/system-init.md).
