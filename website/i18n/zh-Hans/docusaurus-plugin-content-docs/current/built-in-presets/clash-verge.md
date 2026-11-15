---
title: Clash Verge Rev
sidebar_position: 3
---

# Clash Verge Rev

将本地代理、策略组和规则集中保存在一份 `merge.yaml` 中，通过 Clash Verge Rev 的四个订阅
编辑器应用，无需修改远端订阅。本地规则列表也可放在 overlay 中。预设还可通过 mihomo 控制器
请求刷新规则提供者。

内置配置是默认不生效的示例，应用前需按实际需要修改。

## 准备本地配置

授权、更新与恢复方法见[管理应用配置](../guides/app-presets.md)。

先安装 Bun，再将预设复制到 overlay：

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/clash-verge
shine preset overlay link .
```

例如，使用以下 `app/clash-verge/merge.yaml`，让 `example.com` 及其子域名通过局域网
SOCKS5 代理访问。请替换示例地址、端口和域名：

```yaml title="app/clash-verge/merge.yaml"
proxies:
  - { name: LAN-SOCKS, type: socks5, server: 192.0.2.10, port: 1080 }
proxy-groups:
  - { name: LAN PROXY, type: select, proxies: [LAN-SOCKS, DIRECT] }
prepend-rules:
  - DOMAIN-SUFFIX,example.com,LAN PROXY
```

`prepend-rules` 将本地规则放在订阅规则之前。删除其他无需定制的复制文件，不要修改
`~/.shine/clash-verge/` 中的受管副本。

需要规则列表时，从内置 `rule-providers` 示例中选择一种来源：mihomo 的 `HomeDir` 内的
本地文件、同设备 HTTP，或远程 HTTPS，并启用相应策略组与 `prepend-rules`。只有本地文件方式
使用 `HomeDir/ruleset/shine-source/`，通过 overlay 定制这些列表。`proxy: DIRECT` 只影响下载，
服务器需要代理时应调整它。私有域名依赖系统 split DNS 时，还需配置 mihomo 的 `dns.nameserver-policy`。

## 安装并应用

```bash
shine app install clash-verge --dry-run
shine app install clash-verge
```

首次使用时，在 Clash Verge Rev 当前订阅中依次打开并保存 **Extend Config**、**Edit Rules**、
**Edit Proxies**、**Edit Groups** 四个编辑器。Apply 会用 `merge.yaml` 生成的配置替换这四处内容，
请先备份其中已有的定制配置，再运行：

```bash
shine app artifact apply clash-verge
```

升级改动此类别时，也会通过已批准的脚本更新编辑器文件。每次编辑器内容变化后，
在 Clash Verge Rev 中重新选择订阅。远端订阅和编辑器绑定保持不变。

使用上面的 SOCKS5 示例时，将 Clash Verge Rev 设为规则模式，并在 `LAN PROXY` 中选择
`LAN-SOCKS`；选择 `DIRECT` 则改为直连。

## 刷新与移除

需要立即刷新时，在 Shine 环境设置中配置 `CLASH_CONTROLLER_URL` 和 `CLASH_CONTROLLER_TOKEN`，
再运行 `shine app artifact apply clash-verge`；刷新失败后也用此命令重试。未配置 URL 时，
规则提供者仍按自身间隔更新。不要将令牌写入 overlay 或文档。

立即刷新会处理 `merge.yaml` 声明的全部规则提供者。全部刷新成功后，会关闭当前 mihomo 连接，
让应用重连并按新规则匹配，可能短暂中断下载或其他长连接。

`shine app artifact remove clash-verge` 不会清除 Clash Verge Rev 保存的订阅绑定；完全移除时，
还需手动清空上述四个编辑器。
