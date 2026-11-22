---
title: Shell utilities
sidebar_position: 7
---

# Shell utilities

The `utils` preset provides commands for importing Shine values, synchronizing terminal themes,
and copying file contents to the clipboard.

## Install

Install the category, then open a new shell:

```bash
shine shell install utils
```

Run the installed command names, not the scripts inside the preset directory.

## Export a value {#export-a-value}

```bash
shine-env-export MY_TOKEN --as API_TOKEN
```

This loads a plaintext Shine value or decrypts `<KEY>_SECRET` into the current shell. Exported
values remain available to later commands; use [one-command injection](../guides/environment.md#provide-values-to-one-command)
when only one program needs them. See [environment secrets](../guides/environment.md#encrypt-values-with-gpg)
for encryption setup and export limits.

## Apply the terminal theme {#terminal-theme}

```bash
shine-theme-sync
```

This applies `SHINE_TERMINAL_THEME` and `BAT_THEME` to the current session without an extra `eval`.
For detection behavior, automatic profile synchronization, and SSH forwarding, see
[Synchronize the terminal theme](../guides/terminal-theme-sync.md).

## Copy a file to the clipboard

```bash
copyfile ./notes.txt
```

`copyfile` sends file content to the local clipboard through OSC 52. It is Unix-only, and your
terminal or multiplexer must allow OSC 52.
