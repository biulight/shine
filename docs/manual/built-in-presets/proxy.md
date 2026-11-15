---
title: Proxy commands
sidebar_position: 6
---

# Proxy commands

The `proxy` preset provides commands to set and clear proxy settings for the terminal and package managers.

## Install and use

Point the preset at an existing proxy server through [Shine's environment settings](../guides/environment.md#inspect-and-set-values).
For a local server with HTTP on port `7890` and SOCKS5 on `7891`:

```bash
shine env set PROXY_HOST 127.0.0.1
shine env set HTTP_PROXY_PORT 7890
shine env set SOCKS5_PROXY_PORT 7891
shine env set PROXY_NO_PROXY 'localhost,127.0.0.1,::1'
shine shell install proxy
```

Replace the address and ports with your server's values. The preset does not provide a proxy server.
Open a new shell after installation, then choose an operation:

| Operation | Command |
| --- | --- |
| Set proxies automatically, preferring SOCKS5 | `setproxy` |
| Use HTTP explicitly | `setproxy http` |
| Use SOCKS5 explicitly | `setproxy sock5` |
| Clear the proxy-related session variables and Yarn proxy entries | `usetproxy` |

When Yarn is installed, `setproxy` also changes its persistent proxy settings; closing the terminal
does not clear those settings.

`usetproxy` clears fixed variable and setting names, including npm/pnpm registry variables; it does
not track who set them or restore previous values. `setproxy` uses the HTTP endpoint for npm/pnpm
and Yarn even in SOCKS5 mode, and sets npm/pnpm's session registry to `https://registry.npmjs.org/`.

`PROXY_NO_PROXY` controls the `NO_PROXY` and `no_proxy` values set by `setproxy` and defaults to
`localhost,127.0.0.1,::1`. After changing it or another proxy variable, `shine update` marks the
installed `proxy` shell preset as updatable; run `shine upgrade` to apply the value.
