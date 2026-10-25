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

Shine first uses `SHINE_TERMINAL_THEME` or `COLORFGBG` if set, then tries to detect the terminal
background. If it cannot determine the theme, it leaves variables unchanged. `--quiet` hides
non-essential messages but still reports configuration errors.

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

`shine ssh <HOST>` forwards the local terminal theme to the remote session, so tools such as `bat`
can stay consistent with your local terminal.
