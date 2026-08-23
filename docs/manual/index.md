---
title: Shine
slug: /
sidebar_position: 1
description: Turn everyday scripts and configuration into personal tools you can install, update, and remove cleanly.
---

# Shine

Turn the scripts and configuration you use every day into personal tools you can install, update,
and remove cleanly.

You may already sync those files between machines. But after they arrive, scripts still need to be
added to `PATH`, application configuration still needs the right destination, and local values
should not travel inside shared files. Updating everything by hand also makes it hard to know what
will be overwritten. Personal configuration also ends up scattered across shell files, application
directories, and system-specific paths, which makes it difficult to maintain, reuse, or share.

Shine brings your scripts, personal configuration, and their installation rules together in a
**Preset**. Maintain and share the preset folder in one place; Shine installs each item where it
belongs. Install only what you need, see what changed before updating, and remove it later without
deleting unrelated files.

**Give personal automation a reviewable lifecycle.**

This manual applies to **Shine 1.6.0**.

## What Shine helps you do

- **Use a script like any other command.** Install it once and call it by name from `PATH`.
- **Keep personal configuration together.** Maintain it in one preset folder; Shine copies,
  transforms, or merges each file where its application expects it.
- **Keep each machine's values on that machine.** A preset declares the keys it needs; you provide
  the values locally.
- **Look before you update.** By default, inspect what changed first; Shine applies it only when you
  choose to upgrade.
- **Remove only what Shine installed.** Your source folder and unrelated files stay in place.

## Try a built-in preset

Install the image helpers, resize a photo, and inspect what Shine added:

```bash
shine list --available
shine install shell/image-tools
img-resize photo.jpg
shine info shell/image-tools
```

You can also browse ready-made configuration for tools such as Starship, Git, Vim, and Ghostty.
Surge and Clash Verge Rev have their own guided setup in [application presets](./guides/app-presets.md).

## Build presets around your routines

A preset folder can arrive through any folder-sync tool, archive, network transfer, version-control
checkout, or manual copy. Shine does not prescribe how you share it.

The built-in `image-tools` preset shows this pattern with reusable compression, resizing, and format
conversion commands. Your own presets might package collision-aware batch renaming, spreadsheet
cleanup, or document printing in the same way. Each command still needs its application or runtime
on the machine. See [custom presets](./guides/custom-presets.md) for the mechanism and the complete
image workflow.

## Keep plaintext secrets out of AI agent workspaces

AI coding agents such as Claude Code and Codex can read workspace files, run commands, and inspect
their output. Shine can seal selected workspace secrets as commit-ready GPG or age ciphertext, then
decrypt and inject them only into the child process that needs them:

```bash
shine env secret seal
shine env run --mode development -- bun run build
```

This reduces the chance that plaintext secrets reach project files, logs, patches, or agent context.
It is not a sandbox: a command that can inspect its environment can still read the injected values.
See [protect environment secrets when using AI agents](./guides/agent-secret-safety.md) for the
workflow and its security boundaries.

## Use the same toolkit for more than files

Save commands you repeat as [tasks](./guides/tasks-and-serve.md), carry selected values into an
[SSH session](./guides/ssh-transfer.md), and decide exactly which remote workflow may receive a
secret. Shine can also prepare selected parts of macOS, Ubuntu, or Windows, but it does not take over
third-party tool versions; see [system initialization](./guides/system-init.md) for that boundary.

## Start here

1. [Install Shine](./installation.md).
2. [Complete your first preset installation](./quick-start.md).
3. Browse the [built-in presets](./reference/built-in-presets.md) to see what you can use now.
4. Continue with [shell presets](./guides/shell-presets.md),
   [application presets](./guides/app-presets.md),
   [system initialization](./guides/system-init.md),
   [terminal theme synchronization](./guides/terminal-theme-sync.md),
   [tasks and the local service](./guides/tasks-and-serve.md), or
   [SSH sessions, secret brokering, and file transfer](./guides/ssh-transfer.md).

If something is already failing, go directly to [Troubleshooting](./troubleshooting.md). Open the
[command reference](./reference/commands.md) when you need every option.
