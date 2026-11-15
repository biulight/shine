---
title: 私有域名解析
sidebar_position: 9
---

# 私有域名解析

`split-dns` 为私有域名后缀配置专用 DNS，支持 macOS、Ubuntu 和 Windows。
设置后缀与服务器地址后，可通过 Shine 预览、应用或移除配置。

## 准备与应用

此项目需要管理员权限，不包含在默认安装组合中。DNS 服务器必须已能访问，ZeroTier 是实现连通的一种方式。

在 [Shine 环境设置](../guides/environment.md#查看和设置变量)中配置后缀和服务器地址，替换以下示例值：

```bash
shine env set PRIVATE_DNS_DOMAIN home.example.internal
shine env set PRIVATE_DNS_SERVERS 10.0.0.53
```

先预览，再应用或移除：

```bash
shine sys info split-dns
shine sys apply split-dns --dry-run
shine sys apply split-dns
shine sys uninstall split-dns --dry-run
```

Ubuntu 上，`split-dns` 要求本机 DNS 查询经过 `systemd-resolved` 的 `127.0.0.53`。
如果 Shine 提示 DNS stub 被关闭，请先重新启用 `DNSStubListener`，或确认本机解析链路会经过
`systemd-resolved`，再应用配置。

它不会安装 ZeroTier、CoreDNS 或创建 DNS 区域；应先确保私有网络和 DNS 服务可用。完整的跨平台网络与排错步骤见[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns)。

通用操作与恢复方法见[管理系统设置](../guides/system-init.md#受管系统项目)。
