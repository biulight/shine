---
title: Private split DNS
sidebar_position: 9
---

# Private split DNS

`split-dns` configures a dedicated DNS server for a private domain suffix on macOS, Ubuntu, and
Windows. Set the suffix and server, then use Shine to preview, apply, or remove the setting.

## Prepare and apply

This item requires administrator access and is not included in default profiles. The DNS server
must already be reachable; ZeroTier is one way to provide that connection.

Set the suffix and server address in [Shine's environment settings](../guides/environment.md#inspect-and-set-values),
replacing these example values:

```bash
shine env set PRIVATE_DNS_DOMAIN home.example.internal
shine env set PRIVATE_DNS_SERVERS 10.0.0.53
```

Preview before applying or removing:

```bash
shine sys info split-dns
shine sys apply split-dns --dry-run
shine sys apply split-dns
shine sys uninstall split-dns --dry-run
```

On Ubuntu, `split-dns` requires local DNS queries to pass through the `127.0.0.53`
systemd-resolved stub. If Shine reports that the stub is disabled, re-enable `DNSStubListener`
or confirm that local resolution passes through systemd-resolved before applying the configuration.

It does not install ZeroTier or CoreDNS or create DNS zones. Set up the private network and DNS first.
See the Chinese guide
[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns).

General operation and recovery: [Manage system settings](../guides/system-init.md#managed-system-items).
