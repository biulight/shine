---
title: 概览与选择
sidebar_position: 1
slug: /built-in-presets
---

# 概览与选择

Shine 随附了一些作者整理的常用脚本、应用配置和系统安装方案。这些预设按作者的日常使用习惯
编写，可按需选用或修改。

## Shell 预设

<div className="built-in-presets-shell-table" aria-hidden="true" />

| 类别 | 命令 | 用途与关键前提 |
| --- | --- | --- |
| `agent` | [`ccenv`](./agent.md) | 启动 Claude Code 并选择 provider 和模型；可使用已配置的 Shine 加密凭据，避免在启动文件中长期明文保存密钥。需要 Bun 和 Claude Code。 |
| `image-tools` | [`img-compress`](./image-tools.md) | 批量压缩图片并保留原图，输出不保证更小。 |
| `image-tools` | [`img-resize`](./image-tools.md) | 按指定尺寸批量缩放图片，不放大或覆盖原图。 |
| `image-tools` | [`img-convert`](./image-tools.md) | 批量转换图片格式，保留原图。 |
| `proxy` | [`setproxy`](./proxy.md) | 设置终端与包管理器代理。需要已有代理服务器，也会修改 Yarn 持久设置。 |
| `proxy` | [`usetproxy`](./proxy.md) | 清除代理相关的会话变量和 Yarn 代理条目，不恢复原值。 |
| `utils` | [`copyfile`](./utils.md#复制文件到剪贴板) | 将文件内容复制到本地剪贴板。仅 Unix，终端或终端复用器须允许 OSC 52。 |
| `utils` | [`shine-env-export`](./utils.md#export-a-value) | 将 Shine 变量导入当前 Shell，需要时解密；导出的值会留在该会话中。 |
| `utils` | [`shine-theme-sync`](./utils.md#terminal-theme) | 将终端主题变量应用到当前会话。 |

图片工具支持 JPEG、PNG、WebP，需要 Bun 1.3.14 或更高版本。

## App 预设

这些预设提供应用配置，应用本身需另行安装。

<div className="built-in-presets-app-table" aria-hidden="true" />

| 类别 | 内容与选用要点 |
| --- | --- |
| `archey4` | 主机、CPU、内存、磁盘与网络摘要布局。 |
| [`clash-verge`](./clash-verge.md) | 订阅之外的本地代理、策略组与规则。需要 Bun；默认示例不生效，需定制后接入订阅编辑器。 |
| `docker-desktop` | Docker Desktop 及容器代理设置。仅 Windows；替换整个 `proxy` 和 `containersProxy` 配置块，保留块外设置。需重启 Docker Desktop。 |
| `docker-engine` | 根据 Shine 变量生成 daemon 代理设置。替换整份 `daemon.json`，先审阅现有设置；Unix 需要管理员权限，需重启 daemon。 |
| `fastfetch` | 硬件、系统、网络与外观摘要布局。 |
| `ghostty` | 明暗外观与分屏快捷键，包括 Ctrl + H/J/K/L 切换分屏。 |
| `git` | 状态、分支、图形日志别名与 Git 默认设置。启用拉取时 rebase、推送当前分支。 |
| `JetBrains` | IdeaVim 编辑与分屏导航映射。需要启用 IdeaVim 插件。 |
| `starship` | 时间、Git 信息和命令结果等提示符设置。需要在 Shell 中启用 Starship。 |
| [`surge`](./surge.md) | 本地代理、策略组与规则，可选 URI 订阅。macOS；需要 Bun 和正在运行、CLI 支持模块管理的 Surge，应用前需定制本地配置。 |
| `vim` | 编辑、搜索、缩进、按键配置及本机覆盖文件。需检查主题与插件要求。 |

## 系统预设

以下组合用于安装选定的开发工具，已完成的安装会被跳过；bootstrap 不用于日常软件更新。`mise` 项目只安装并启用 mise，
工具版本由 mise 自行配置。

<div className="built-in-presets-system-table" aria-hidden="true" />

| 平台 | Profile | 包含项目 |
| --- | --- | --- |
| macOS | `required` | Homebrew、Yazi、Starship。 |
| macOS | `recommended`（默认） | `required` 加 Rust、Neovim、AstroNvim、ZeroTier、Zsh 插件、zoxide、Atuin、fzf、bat、eza。 |
| macOS | `all` | `recommended` 加 nvm、Bun、pnpm、mise、Fastfetch。 |
| Ubuntu | `recommended`（默认） | Rust、Neovim、AstroNvim、Atuin、Yazi、Starship、zoxide、zsh-vi-mode、fzf、bat、eza、Docker Engine。 |
| Ubuntu | `all` | `recommended` 加 ZeroTier、Bun、pnpm、mise、Homebrew。 |
| Ubuntu | `minimal` | Neovim、fzf、bat、eza、zoxide；适合不需要历史同步、提示符与 JavaScript 工具链的服务器。 |
| Arch Linux | `minimal`（默认） | 通过 pacman 安装 Neovim、fzf、bat、eza、zoxide，支持 Bash/Zsh 集成。仅支持官方 x86_64 Arch Linux。 |
| Windows | `required` | Rust、Yazi、Starship。 |
| Windows | `recommended`（默认） | `required` 加 Neovim、zoxide、Atuin、fzf、bat、eza、ZeroTier。 |
| Windows | `all` | `recommended` 加 Bun、pnpm、mise。 |

平台前提与安装后步骤见[系统安装方案](./system-bootstrap.md)；Arch 安装前需要
[完成全系统更新](./system-bootstrap.md#arch-linux-preparation)。
[私有域名解析](./split-dns.md) 是这些组合之外的可选项目，支持 macOS、Ubuntu 和 Windows，
需要管理员权限和可访问的 DNS 服务器。

## 平台支持 {#platform-support}

<!-- BEGIN GENERATED PRESET PLATFORM CAPABILITIES -->
| Preset capability | macOS | Linux | Windows |
| --- | --- | --- | --- |
| `app/JetBrains` | ✓ | ✓ | ✓ |
| `app/archey4` | ✓ | ✓ | — |
| `app/clash-verge` | ✓ | ✓ | ✓ |
| `app/docker-desktop` | — | — | ✓ |
| `app/docker-engine` | ✓ | ✓ | ✓ |
| `app/fastfetch` | ✓ | ✓ | ✓ |
| `app/ghostty` | ✓ | ✓ | — |
| `app/git` | ✓ | ✓ | ✓ |
| `app/starship` | ✓ | ✓ | ✓ |
| `app/surge` | ✓ | — | — |
| `app/vim` | ✓ | ✓ | ✓ |
| `shell/agent/ccenv` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-compress` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-convert` | ✓ | ✓ | ✓ |
| `shell/image-tools/img-resize` | ✓ | ✓ | ✓ |
| `shell/proxy/setproxy` | ✓ | ✓ | ✓ |
| `shell/proxy/usetproxy` | ✓ | ✓ | ✓ |
| `shell/utils/copyfile` | ✓ | ✓ | — |
| `shell/utils/shine-env-export` | ✓ | ✓ | ✓ |
| `shell/utils/shine-theme-sync` | ✓ | ✓ | ✓ |
<!-- END GENERATED PRESET PLATFORM CAPABILITIES -->

## 使用与定制 {#use-and-customize}

安装、更新、移除、权限和恢复说明见 [Shell](../guides/shell-presets.md)、
[App](../guides/app-presets.md) 或[系统](../guides/system-init.md)指南。
各预设的专项设置可从上表链接进入。

修改内置默认配置可使用 [overlay](../guides/custom-presets.md#使用-overlay-覆盖少量文件)，
变量设置见[环境指南](../guides/environment.md)。
