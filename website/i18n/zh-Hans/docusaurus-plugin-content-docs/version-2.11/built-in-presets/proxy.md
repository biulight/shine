---
title: 代理命令
sidebar_position: 6
---

# 代理命令

`proxy` 预设提供设置和清除终端、包管理器代理的命令。

## 安装与使用

通过 [Shine 环境设置](../guides/environment.md#查看和设置变量)指定已有代理服务器。
例如，本机代理的 HTTP 端口为 `7890`、SOCKS5 端口为 `7891`：

```bash
shine env set PROXY_HOST 127.0.0.1
shine env set HTTP_PROXY_PORT 7890
shine env set SOCKS5_PROXY_PORT 7891
shine env set PROXY_NO_PROXY 'localhost,127.0.0.1,::1'
shine shell install proxy
```

请按服务器实际配置替换地址和端口；预设不提供代理服务器。安装后打开新的 Shell，按需选择操作：

| 操作 | 命令 |
| --- | --- |
| 自动设置代理，优先 SOCKS5 | `setproxy` |
| 显式使用 HTTP | `setproxy http` |
| 显式使用 SOCKS5 | `setproxy sock5` |
| 清除代理相关的会话变量与 Yarn 代理条目 | `usetproxy` |

安装了 Yarn 时，`setproxy` 还会修改其持久代理设置；关闭终端不会清除这部分设置。

`usetproxy` 按固定名称清除变量和设置，包括 npm/pnpm 的 registry 变量；不会区分是谁设置的，
也不会恢复原值。即使选择 SOCKS5，`setproxy` 仍为 npm/pnpm 和 Yarn 使用 HTTP 端点，并将
npm/pnpm 的会话 registry 设置为 `https://registry.npmjs.org/`。

`PROXY_NO_PROXY` 控制 `setproxy` 设置的 `NO_PROXY` 和 `no_proxy`，默认为
`localhost,127.0.0.1,::1`。修改它或其他代理变量后，`shine update` 会把已安装的
`proxy` shell 预设标记为可更新；运行 `shine upgrade` 应用新值。
