---
title: Surge
sidebar_position: 2
---

# Surge

Surge 预设将本地代理、策略组和规则保存在独立文件中，通过 apply 接入 Profile。
General 设置通过原生模块叠加，不改写 `[General]`。它还可从 URI 订阅或 wGetCloud 本地缓存生成代理文件，
并在刷新内容变化后重新加载 Surge。

## 准备本地配置

授权、更新与恢复方法见[管理应用配置](../guides/app-presets.md)。

请先安装 Bun，并使用 CLI 支持模块管理的 Surge Mac。执行 artifact apply 和 remove 时，Surge
必须正在运行。先将预设复制到 overlay：

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
shine preset overlay link .
```

例如，让 `example.com` 及其子域名通过局域网 SOCKS5 代理访问。替换示例地址、端口和域名，
在 overlay 中三个文件的对应节内加入以下条目：

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

删除无需定制的复制文件。修改 overlay 源文件，不要修改 Surge Profiles 目录中的受管副本。

改用规则列表时，按需调整内置文件中 `LAN Network`、`LAN PROXY`、`Other Direct` 的注释示例。
每类流量只启用一种来源：相对 `rules/*.list` 文件、同设备 HTTP，或替换为自己域名的远程 HTTPS。
相对文件通常最简单。`localhost` 指运行 Surge 的设备，在 iOS 上也不会指向另一台局域网主机。

## 可选：General 设置

需要定制 `[General]` 时，在 overlay 中保留并编辑 `app/surge/shine-general.sgmodule`：

```ini
#!name=shine-general
#!desc=Local General settings managed by Shine.
#!system=mac

[General]
skip-proxy = %APPEND% *.internal
```

保留文件名、`#!name` 和原生目标目录 `~/Library/Application Support/Surge/Profiles/` 不变。
内置模块只有注释示例。`%APPEND%` 追加列表值但不去重，`key = value` 覆盖标量设置。
`skip-proxy` 控制系统代理绕过设置，不等同于 `DIRECT` 路由规则，增强模式仍可能接管相关流量。

模块的启用状态属于当前设备，不局限于 `SURGE_PROFILE`，删除后重新安装也不会重置。

## 可选订阅 {#surge-uri-subscriptions}

预设支持 HTTPS Base64 URI 订阅中的兼容 `ss://`、`vmess://` 和 `trojan://` 记录，Trojan 支持
TCP 和 WebSocket。VLESS、不支持的传输或安全模式、插件、坏记录及重复项会被跳过；诊断不含凭据，
自己的 `local-proxies.conf` 会保留。配置 HTTPS 输入时，请使用服务商的 URI/Base64 订阅地址。

按需设置对应订阅的变量：

| 刷新时使用的源路径 | 安装后的文件名 | 环境变量 |
| --- | --- | --- |
| `subscription-proxies.conf` | `subscription-proxies.conf` | `SURGE_SUBSCRIPTION_URL` |
| `sub/wget-cloud-proxies.conf` | `wget-cloud-proxies.conf` | `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` 或 `W_GET_CLOUD_SURGE_CACHE_SOURCE` |

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
```

使用 wGetCloud 的 HTTPS 订阅时，设置 `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL`。
要离线生成，请取消设置该变量，并将 `W_GET_CLOUD_SURGE_CACHE_SOURCE` 设为导入到
`~/Library/Application Support/Surge/Profiles/` 中的配置文件名。Shine 会读取该订阅已有的缓存。
两个变量都设置时，URL 优先。下载失败时保留上次生成的文件。

```bash
shine env set W_GET_CLOUD_SURGE_CACHE_SOURCE WgetCloud.conf
```

也可以填写已下载缓存文件的绝对路径，该文件须包含 `🚀 节点选择` 策略组。

`local-proxy-groups.conf` 中的 `Subscription` 组通过 `policy-path` 读取 `subscription-proxies.conf`。
`WgetCloud` 组读取 `wget-cloud-proxies.conf`。其他策略组可用 `include-other-group=Subscription`
或 `include-other-group=WgetCloud` 复用相应节点。

## 安装并应用

Profile 必须与安装后的本地配置文件位于同一目录，默认是
`~/Library/Application Support/Surge/Profiles/`，且 `[Proxy]`、`[Proxy Group]`、`[Rule]`
三个节中均已有 `#!include` 行。将下例 `Profile.conf` 替换为实际文件名。安装前先审阅 overlay。
已配置的订阅 URL 须在安装期间可访问。

```bash
shine app install surge --dry-run
shine app install surge
shine env set SURGE_PROFILE "~/Library/Application Support/Surge/Profiles/Profile.conf"
shine app artifact apply surge
```

Apply 将本地代理、策略组和规则文件接入 Profile，启用 `shine-general`，再重新加载 Surge。
已启用模块的后续更新会在安装或升级重新加载 Surge 时生效。若 apply 提示已安装模块与预设不一致，
请核对修改并先安装或升级。

使用上面的 SOCKS5 示例时，将 Surge 设为规则模式，并在 `LAN PROXY` 中选择 `LAN-SOCKS`；
选择 `DIRECT` 则改为直连。

## 刷新订阅

日常 `update` 和 `upgrade` 不读取这些订阅。请显式刷新需要的文件。如果 URL 有访问窗口，
先打开访问窗口再刷新。本地 wGetCloud 缓存可离线读取，使用上次下载的节点。

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
```

内容变化后会重新加载 Surge，失败则保留上次成功文件。后续新增订阅时，首次刷新前先运行
`shine app install surge`。本地修改仍会保留，除非明确添加 `--force`。

出现 HTTP 错误、超时、响应过大或无兼容节点时，检查订阅服务，并确认 URL 返回的是
[支持的格式](#surge-uri-subscriptions)。
如果本地缓存报错，请确认配置的 `[Proxy]` 和 `[Proxy Group]` 两节合计只引用一个订阅，且 Surge
已下载该订阅。如果填写的是绝对路径，请确认它指向缓存文件。

## 移除集成

```bash
shine app artifact remove surge
```

该操作停用 `shine-general`、重新加载 Surge 并撤销 Profile 改动，保留已安装文件；App 卸载还会
移除受管文件。其他模块不会被修改。

不要先删除模块文件：其效果可能持续到重新加载，删除也不会重置启用状态。若已删除，请恢复文件，
再运行 remove。

Apply 或 remove 失败、中断时，部分设置可能已改变。修复提示的问题后重复原命令。若旧 Shine
proxy-group 标记缺失或嵌套，请先修复 Profile 中的标记再重试；此时 remove 可能已经停用模块。
