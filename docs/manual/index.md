---
title: Shine
slug: /
sidebar_position: 1
description: Keep a development environment portable, usable, and safe across machines and remote sessions.
---

# Shine

Shine keeps a development environment portable, usable, and safe across machines and remote
sessions. It gives shell commands, application configuration, bootstrap steps, environment values,
tasks, and SSH workflows explicit ownership and reviewable lifecycle operations.

This manual applies to **Shine 1.4.0**.

## Four connected workflows

### Set up and reconcile

Install shell and application presets, inspect pending changes, and safely upgrade or uninstall only
the content Shine manages. Bootstrap scripts help initialize a new macOS, Ubuntu, or Windows machine.

System bootstrap is intentionally not package-version management. It records what a bootstrap run
did and can perform read-only update checks, but third-party tools and package managers remain the
authority for versions and upgrades. A built-in step may install and activate mise; it does not
configure mise or manage runtime versions on mise's behalf.

### Keep terminal work repeatable

Install portable commands such as `setproxy` and `copyfile`, save argv-based personal tasks,
synchronize light/dark terminal themes, and serve generated resources required by managed app
configuration.

### Continue work across SSH

`shine ssh` can forward explicitly selected values, keep terminal theme context, and transfer files
or directories over the authenticated session. The remote side asks the local agent to perform the
transfer, so there is no separate file-transfer daemon to expose.

### Release secrets deliberately

Store workspace secrets with GPG or age, optionally backed by macOS Touch ID. For remote AI and
tooling workflows, the SSH secret broker keeps decryption local and matches requests against an exact
workspace, host, command, and secret policy before releasing values.

## Presets are products and extension points

Built-in presets provide usable defaults, including provider-specific Surge and Clash Verge Rev
artifacts that remove application-specific assembly work. Treat them as starting points: copy one
category, override selected paths with an overlay, or maintain a complete external preset source.
Command output identifies the effective base source and overlay.

Lifecycle commands operate on `app/<category>`, `shell/<category>`, and `sys/<item>`. Individual app
files, shell commands, scripts, drivers, and receipts remain visible through `info`, status details,
and `--diff`; they are not separate default install/upgrade units.

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
