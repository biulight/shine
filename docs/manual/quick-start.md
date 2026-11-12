---
title: Quick start
sidebar_position: 3
---

# Quick start

This walkthrough uses the proxy shell preset to browse, install, use, and preview removal of a
preset. First [install Shine](./installation.md) and have an HTTP proxy service available.
This preset configures access to a proxy; it does not install or start one. If you do not use a
proxy, start with [application presets](./guides/app-presets.md) instead.

## 1. Browse available presets

```bash
shine shell list
shine app list
shine sys list
shine list --available
```

`list --available` also accepts `app`, `shell`, or `sys` to filter it.

## 2. Install the proxy commands

Check the proxy address before installing. The default is `127.0.0.1:6152`; if yours differs,
set the actual host and port with `shine env set PROXY_HOST <HOST>` and
`shine env set HTTP_PROXY_PORT <PORT>` first, then install. See the
[environment guide](./guides/environment.md) for configuration details.

Shine shows the intended changes and required permissions and defaults to No. Review them and
confirm to install the commands.

```bash
shine shell install proxy
```

Open a new terminal or reload the current shell configuration:

```bash
source ~/.zshrc
```

For Bash, run `source ~/.bashrc`.

PowerShell users can reopen the terminal to load the updated profile.

## 3. Use and inspect the preset

```bash
setproxy http
shine list
shine info shell/proxy
```

`setproxy http` configures an HTTP proxy for subsequent commands in this terminal. If Yarn is
installed, it also changes Yarn's persistent proxy settings; closing the terminal does not clear those.

Disable the session proxy and clear those Yarn proxy settings:

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

- turn a personal script or application configuration into a [custom
  preset](./guides/custom-presets.md);
- install [application configuration](./guides/app-presets.md), such as Surge or
  Clash Verge Rev;
- initialize a machine with the [system setup](./guides/system-init.md);
- save [repeatable tasks](./guides/tasks-and-serve.md); or
- continue work through [SSH environment forwarding and file transfer](./guides/ssh-transfer.md).
