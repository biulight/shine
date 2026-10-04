---
title: Synchronize the terminal theme
sidebar_position: 7
---

# Synchronize the terminal theme

In managed shell profiles on macOS and Ubuntu, Shine can detect whether the terminal background is
light or dark and export `SHINE_TERMINAL_THEME=light|dark`. If `BAT_THEME` is not already set, it
also selects `GitHub` for a light background or `OneHalfDark` for a dark background.

## Check manually

```bash
shine theme sync
eval "$(shine theme sync --quiet)"
```

The first command only displays the shell `export` statements. The second applies them to the
current shell. Shine never overwrites a `BAT_THEME` you set yourself.

Shine checks an existing `SHINE_TERMINAL_THEME`, then terminal-provided `COLORFGBG`, and only then
queries the background through OSC 11. If it cannot determine the appearance, it does not guess or
write variables.

## Synchronize in a managed profile

After `shine sys bootstrap`, managed macOS and Ubuntu profiles invoke synchronization automatically.
Disable it through either mechanism:

```toml title="~/.shine/config.toml"
sync_terminal_theme = false
```

```bash
export SHINE_SYNC_TERMINAL_THEME=0
```

The environment variable has higher precedence and is useful for temporary overrides. Manual
`shine theme sync` remains available even when automatic synchronization is disabled.

You can also install the `shine-theme-sync` command from the `utils` category for a profile you
manage yourself:

```bash
shine shell install utils
eval "$(shine-theme-sync)"
```

## SSH sessions

Before connecting, `shine ssh <HOST>` reads the local terminal appearance and injects it into the
remote session. The remote terminal therefore does not need to answer an OSC 11 query, and tools such
as `bat` can stay consistent with the local terminal.
