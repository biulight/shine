---
title: Clash Verge Rev
sidebar_position: 3
---

# Clash Verge Rev

Keep local proxies, groups, and rules in one `merge.yaml`, then apply them through Clash Verge Rev's
four subscription editors without changing the remote subscription. Local rule lists can also live
in the overlay. The preset can request a rule-provider refresh through mihomo's controller.

The bundled configuration is an inactive example; customize it before applying.

## Prepare local configuration

For authorization, updates, and recovery, see [Manage application configuration](../guides/app-presets.md).

Install Bun, then copy the preset to an overlay:

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/clash-verge
shine preset overlay link .
```

For example, use this `app/clash-verge/merge.yaml` to route `example.com` and its subdomains through
a LAN SOCKS5 proxy. Replace the example address, port, and domain:

```yaml title="app/clash-verge/merge.yaml"
proxies:
  - { name: LAN-SOCKS, type: socks5, server: 192.0.2.10, port: 1080 }
proxy-groups:
  - { name: LAN PROXY, type: select, proxies: [LAN-SOCKS, DIRECT] }
prepend-rules:
  - DOMAIN-SUFFIX,example.com,LAN PROXY
```

`prepend-rules` places local rules before subscription rules. Delete other copied files you do not
customize; do not edit the managed copy under `~/.shine/clash-verge/`.

For rule lists, choose one of the bundled `rule-providers` examples: local files inside mihomo's
`HomeDir`, HTTP on the same device, or remote HTTPS. Enable the matching policy groups and
`prepend-rules`. Only the local-file layout uses `HomeDir/ruleset/shine-source/`; customize those
lists through the overlay. `proxy: DIRECT` affects downloads only; adjust it if the server needs
a proxy. Private domains relying on system split DNS also need mihomo `dns.nameserver-policy` configuration.

## Install and apply

```bash
shine app install clash-verge --dry-run
shine app install clash-verge
```

For first use, open and save the current subscription's **Extend Config**, **Edit Rules**,
**Edit Proxies**, and **Edit Groups** editors in Clash Verge Rev. Apply replaces all four editors'
contents with the configuration generated from `merge.yaml`; back up existing customizations first.
Then run:

```bash
shine app artifact apply clash-verge
```

Upgrades that change this category also update the editor files through the approved script.
After editor content changes, reselect the subscription in Clash Verge Rev. The remote subscription
and editor bindings remain unchanged.

To use the SOCKS5 example above, switch Clash Verge Rev to Rule mode and select `LAN-SOCKS`
in `LAN PROXY`; select `DIRECT` to connect directly instead.

## Refresh and remove

For immediate refresh, set `CLASH_CONTROLLER_URL` and `CLASH_CONTROLLER_TOKEN` in Shine's environment
settings, then run `shine app artifact apply clash-verge` again. Use the same command to retry a
failed refresh. Without the URL, providers still update at their own intervals. Keep the token out
of overlays and documentation.

Immediate refresh includes all rule providers declared in `merge.yaml`. Once all refreshes succeed,
it closes current mihomo connections so applications reconnect under the new rules. This can briefly
interrupt downloads and other long-lived sessions.

`shine app artifact remove clash-verge` does not clear subscription bindings stored by Clash Verge
Rev. For complete removal, manually clear the four editors as well.
