---
title: Shine
slug: /
sidebar_position: 1
description: Manage shell commands, application configuration, and system initialization presets with Shine.
---

# Shine

Shine is a cross-platform command-line tool that packages common shell scripts, application
configuration, and system initialization steps as presets you can install, inspect, upgrade, and
safely uninstall.

This manual applies to **Shine 1.3.0**.

## What you can do with Shine

- Install shell commands such as `setproxy` and `copyfile`, including automatic command-path setup.
- Install configuration for applications such as Git, Starship, Ghostty, Vim, and Docker.
- Select and run system initialization steps on macOS, Ubuntu, and Windows.
- Check, without modifying the system, whether software installed by system initialization has
  updates, then decide whether to upgrade it yourself.
- Export built-in presets and keep customizations in an external directory or overlay.
- Deploy external shell presets as snapshots by default, with an explicit live mode for preset
  development.
- Manage configuration values and sensitive workspace environment variables with GPG.
- Install transparent wrappers for commands that need fixed environment variables, injecting only
  allow-listed secrets into that command's child process.
- Manage shared team secrets with `age` and optional macOS Touch ID support.
- Transfer files or directories between the local and remote hosts in a `shine ssh` session.
- Save personal commands and generate helper resources that application presets can expose through
  a local HTTP service.
- Explicitly refresh supported URI subscriptions into Shine-managed Surge policy files without
  consuming short-lived access windows during routine status checks.
- Detect the terminal light or dark appearance and provide consistent theme variables to `bat` and
  remote SSH sessions.
- Compare installed files with their presets and upgrade only content managed by Shine.
- Browse, inspect, and update resources through consistent `app/`, `shell/`, and `sys/` targets.

## Start here

1. [Install Shine](./installation.md).
2. [Complete your first preset installation](./quick-start.md).
3. Review the [built-in presets](./reference/built-in-presets.md) to confirm categories, platforms,
   destinations, and prerequisites.
4. Continue with [shell presets](./guides/shell-presets.md),
   [application presets](./guides/app-presets.md),
   [system initialization](./guides/system-init.md),
   [terminal theme synchronization](./guides/terminal-theme-sync.md),
   [tasks and the local service](./guides/tasks-and-serve.md), or
   [SSH sessions, secret brokering, and file transfer](./guides/ssh-transfer.md).

If something is already failing, go directly to [Troubleshooting](./troubleshooting.md). See the
[command reference](./reference/commands.md) for the complete command surface.
