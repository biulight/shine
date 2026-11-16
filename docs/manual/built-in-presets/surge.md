---
title: Surge
sidebar_position: 2
---

# Surge

The Surge preset keeps local proxies, policy groups, and rules in separate files and connects them
to your Profile with apply. General settings use a native module without rewriting `[General]`.
It can also generate proxy files from URI subscriptions or a local wGetCloud cache and reload Surge when refreshed
content changes.

## Prepare local configuration

For authorization, updates, and recovery, see [Manage application configuration](../guides/app-presets.md).

Install Bun and use Surge Mac with CLI module support. Surge must be running for artifact apply
and remove. Copy the preset to an overlay:

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
shine preset overlay link .
```

For example, route `example.com` and its subdomains through a LAN SOCKS5 proxy. Replace the example
address, port, and domain, and add these entries to the corresponding sections in the overlay files:

```ini title="app/surge/local-proxies.conf"
[Proxy]
LAN-SOCKS = socks5, 192.0.2.10, 1080
```

```ini title="app/surge/local-proxy-groups.conf"
[Proxy Group]
LAN PROXY = select, LAN-SOCKS, DIRECT
```

```ini title="app/surge/local-rules.conf"
[Rule]
DOMAIN-SUFFIX,example.com,LAN PROXY
```

Delete copied files you do not customize. Edit overlay sources rather than the managed copies in
Surge's Profiles directory.

For a rule-list setup instead, adapt the commented `LAN Network`, `LAN PROXY`, and `Other Direct`
examples in the bundled files. For each traffic class, enable one source: relative `rules/*.list` files, HTTP on the same
device, or remote HTTPS with your own domain. Relative files are usually simplest. `localhost`
means the device running Surge, including on iOS; it does not point to another LAN host.

## Optional General settings

To customize `[General]`, keep and edit `app/surge/shine-general.sgmodule` in the overlay:

```ini
#!name=shine-general
#!desc=Local General settings managed by Shine.
#!system=mac

[General]
skip-proxy = %APPEND% *.internal
```

Keep the filename, `#!name`, and native destination `~/Library/Application Support/Surge/Profiles/`
unchanged. The built-in module contains only commented examples. `%APPEND%` adds list values
without deduplication; `key = value` overrides a scalar setting. `skip-proxy` controls system proxy
bypass, not a `DIRECT` routing rule; Enhanced Mode may still handle that traffic.

The module's enabled state is device-wide, not limited to `SURGE_PROFILE`, and survives deletion
and reinstallation.

## Optional subscriptions {#surge-uri-subscriptions}

The preset accepts HTTPS Base64 URI subscriptions with compatible `ss://`, `vmess://`, and
`trojan://` records; Trojan supports TCP and WebSocket. VLESS, unsupported transports or security
modes, plugins, malformed records, and duplicates are skipped. Diagnostics omit credentials,
and your `local-proxies.conf` is preserved. For an HTTPS input, use the provider's URI/Base64 URL.

Set the variable for each subscription you use:

| Source path for refresh | Installed filename | Environment variable |
| --- | --- | --- |
| `subscription-proxies.conf` | `subscription-proxies.conf` | `SURGE_SUBSCRIPTION_URL` |
| `sub/wget-cloud-proxies.conf` | `wget-cloud-proxies.conf` | `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` or `W_GET_CLOUD_SURGE_CACHE_SOURCE` |

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
```

For wGetCloud, set `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` to use the HTTPS subscription.
To generate offline, leave that variable unset and set `W_GET_CLOUD_SURGE_CACHE_SOURCE` to the
filename of the imported configuration in `~/Library/Application Support/Surge/Profiles/`.
Shine reads the existing cache for its subscription. If both variables are set, the URL takes
precedence. A failed download leaves the previous output in place.

```bash
shine env set W_GET_CLOUD_SURGE_CACHE_SOURCE WgetCloud.conf
```

You can also supply the absolute path to a downloaded cache file containing the `🚀 节点选择` group.

The `Subscription` group in `local-proxy-groups.conf` reads `subscription-proxies.conf` through
`policy-path`. The `WgetCloud` group reads `wget-cloud-proxies.conf`. Use
`include-other-group=Subscription` or `include-other-group=WgetCloud` to reuse their nodes.

## Install and apply

The Profile must be beside the installed local files, normally in
`~/Library/Application Support/Surge/Profiles/`, with a `#!include` line in each of `[Proxy]`,
`[Proxy Group]`, and `[Rule]`. Replace `Profile.conf` below with its actual filename. Review the
overlay before installing. Any configured subscription URL must be reachable during installation.

```bash
shine app install surge --dry-run
shine app install surge
shine env set SURGE_PROFILE "~/Library/Application Support/Surge/Profiles/Profile.conf"
shine app artifact apply surge
```

Apply connects the local proxy, group, and rule files to the Profile, enables `shine-general`, and
reloads Surge. Updates to an enabled module take effect when install or upgrade reloads Surge. If apply
reports that the installed module differs from the preset, review your edits and install or upgrade first.

To use the SOCKS5 example above, switch Surge to Rule-based Proxy mode and select `LAN-SOCKS`
in `LAN PROXY`; select `DIRECT` to connect directly instead.

## Refresh subscriptions

Routine `update` and `upgrade` do not read these subscriptions. Refresh the file you use explicitly.
If your URL has an access window, open it before refreshing. A local wGetCloud cache can be
read offline and supplies the nodes from its last download.

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
```

Changed content reloads Surge; failure preserves the last successful file. If you add a subscription
later, run `shine app install surge` before its first refresh. Local edits remain protected unless
you explicitly add `--force`.

For HTTP errors, timeouts, oversized responses, or no compatible nodes, check the subscription
service and confirm that its URL returns the [supported format](#surge-uri-subscriptions).
For a local cache error, check that the named configuration references a single subscription across
`[Proxy]` and `[Proxy Group]` and that Surge has already downloaded it. If you supplied an absolute
path, check that it points to the cache file.

## Remove the integration

```bash
shine app artifact remove surge
```

This disables `shine-general`, reloads Surge, and undoes the Profile changes while keeping installed
files. App uninstall also removes the managed files. Other modules are not changed.

Do not delete the module file first: its effect may remain until reload, and deletion does not
reset its enabled state. If already deleted, restore the file and run remove again.

If apply or remove fails or is interrupted, some settings may already have changed. Fix the reported
issue and repeat the command. If old Shine proxy-group markers are missing or nested, repair them
in your Profile before retrying; remove may already have disabled the module.
