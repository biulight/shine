---
title: Surge
sidebar_position: 2
---

# Surge

用 Shine 的 Surge 预设，可以在继续接收服务商订阅更新的同时，加入自己的代理、策略组和分流规则。
还可以把多个订阅的节点汇集到同一个 Surge 策略组。

请先安装 Bun，并使用[支持 CLI 模块管理的 Surge Mac](https://manual.nssurge.com/tools/cli.html)。
设置或移除集成时，请保持 Surge 运行。

## 准备要定制的配置

1. 在 Surge 的 **Profile** 设置中，从 URL 导入服务商的 Surge 配置。已导入时可跳过此步。
2. 基于这份配置创建一份[链接配置（Linked Profile）](https://manual.nssurge.com/profile/format.html)，
   保存到 Surge 本机的 Profiles 目录。这就是 Shine 要修改的配置。确认 `[Proxy]`、`[Proxy Group]` 和
   `[Rule]` 中各有一条引用服务商配置的 `#!include` 行，Shine 会把本地文件加到这些引用行中。
3. 记下新建配置的 `.conf` 文件名。如需确认，可用 Surge 的 **在访达中显示（Reveal in Finder）**。

## 添加订阅来源（可选） {#surge-uri-subscriptions}

需要额外的节点时，在安装前设置对应的 Shine 变量。

| 来源 | Shine 变量 | Surge 策略组 |
| --- | --- | --- |
| 服务商的 HTTPS Base64 URI 订阅 | `SURGE_SUBSCRIPTION_URL` | `Subscription` |
| wGetCloud 的 HTTPS Base64 URI 订阅 | `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` | `WgetCloud` |
| Surge 已下载的 wGetCloud 缓存 | `W_GET_CLOUD_SURGE_CACHE_SOURCE` | `WgetCloud` |

例如，可以这样设置服务商的 URI 订阅地址。

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
```

两个 URL 变量都使用服务商提供的 URI/Base64 订阅地址。Shine 会导入兼容的 `ss://`、`vmess://` 和
`trojan://` 节点，Trojan 支持 TCP 和 WebSocket。

### 离线使用 wGetCloud 缓存

在 Surge 的 Profile 设置中找到或单独创建一份本地链接配置，通过 `#!include` 直接引用 wGetCloud 的
Surge 配置 URL。`[Proxy]` 和 `[Proxy Group]` 两节合计只能引用这一个 URL。先在 Surge 中载入这份配置，
等订阅下载完成。将 `W_GET_CLOUD_SURGE_CACHE_SOURCE` 设为本机 Profiles 目录中这份**来源配置的文件名**，
例如 `WgetCloud.conf`。

```bash
shine env set W_GET_CLOUD_SURGE_CACHE_SOURCE WgetCloud.conf
```

也可以填已下载的缓存文件的绝对路径。Shine 会导入缓存中直接定义在 `[Proxy]` 节里的节点。离线使用时请让
`W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` 保持未设置。两者都设置时，Shine 优先使用 URL。

## 添加自己的设置（可选）

按 [overlay 设置方法](../guides/custom-presets.md#使用-overlay-覆盖少量文件)为 `app/surge` 创建并关联
overlay，保留要定制的副本，在 overlay 中编辑。

- `local-proxies.conf` 用于添加自己的[代理](https://manual.nssurge.com/policies/overview.html)。
- `local-proxy-groups.conf` 用于定义自己的[策略组](https://manual.nssurge.com/policy-groups/policy-including.html)。
  配置好两个订阅来源后，可在自己的策略组中用 `include-other-group="Subscription,WgetCloud"` 汇集两组节点。
- `local-rules.conf` 用于添加[分流规则](https://manual.nssurge.com/rules/overview.html)，
  Surge 会在服务商的规则之前检查这些规则。
- `shine-general.sgmodule` 用于通过 [Surge 模块](https://manual.nssurge.com/profile/module.html)叠加 General 设置。
  请保留文件名和 `#!name=shine-general`，内置模块里有注释示例。启用后，模块设置在本机范围内生效，
  切换 Profile 后仍适用。

## 安装并应用

把下方的 `MySurge.conf` 换成**要让 Shine 修改的那份链接配置的文件名**。内置预设会把文件安装到命令中所示的
本机 Profiles 目录，Shine 会从目标配置所在目录找到要接入的文件。确认已配置的订阅 URL 可访问，
并在批准前审阅安装和应用计划。

```bash
shine env set SURGE_PROFILE "~/Library/Application Support/Surge/Profiles/MySurge.conf"
shine app install surge --dry-run
shine app install surge
shine app artifact apply surge
```

应用后，Shine 会接入本地文件、启用 `shine-general` 并重新加载 Surge。在 Surge 中选中目标配置，
检查已配置的策略组是否出现，并在规则模式下验证分流规则。审批与更新说明见[管理应用配置](../guides/app-presets.md)。

若应用时提示缺少 `#!include`，请检查目标配置中对应的节。若提示已安装模块与预设不一致，
请核对修改，先安装或升级，再重试。

## 刷新订阅

按已配置的来源选择命令。日常 `update` 和 `upgrade` 会跳过这些订阅。

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
```

后续添加订阅时，首次刷新前先运行 `shine app install surge`。刷新成功且内容有变化时，Shine 会重新加载 Surge。
刷新失败时保留上次成功的文件。除非显式添加 `--force`，刷新会保留本地修改。

- 下载失败或找不到兼容节点时，检查订阅 URL 是否可访问，以及返回的[格式](#surge-uri-subscriptions)是否受支持。
- 离线缓存报错时，检查来源配置的文件名和 URL 引用，并确认 Surge 已下载订阅。若填写了绝对路径，
  确认它指向缓存文件。

## 移除集成

让 `SURGE_PROFILE` 继续指向应用时的那份配置，并在 Surge 运行时执行以下命令。

```bash
shine app artifact remove surge
```

Shine 会移除配置中由它添加的引用、停用 `shine-general` 并重新加载 Surge。请保留模块文件直到移除成功，
以便 Surge 清除其启用状态。若已删除模块文件，先恢复文件再重试。还想清理已安装的文件时，
请参阅 [App 卸载](../guides/app-presets.md#卸载与恢复)。

应用或移除失败、中断时，部分更改可能已经生效。修复提示的问题后重试原命令。
若 Shine 提示旧版策略组标记损坏，请先在目标配置里修复标记。
