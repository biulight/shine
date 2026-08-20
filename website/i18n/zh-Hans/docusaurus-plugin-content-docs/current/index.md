---
title: Shine
slug: /
sidebar_position: 1
description: 使用 Shine 在不同机器与远程会话之间保持开发环境可迁移、可用且安全。
---

# Shine

Shine 用于在不同机器与远程会话之间保持开发环境可迁移、可用且安全。它让 Shell 命令、
应用配置、初始化步骤、环境变量、任务和 SSH 工作流拥有明确的归属与可审阅的生命周期。

当前手册适用于 **Shine 1.4.0**。

## 四条相互衔接的工作流

### 初始化并持续对齐

安装 Shell 与应用预设，先检查待处理变更，再安全升级或卸载 Shine 实际管理的内容；
系统引导脚本则帮助初始化新的 macOS、Ubuntu 或 Windows 机器。

系统引导并不是包版本管理。它记录一次引导执行的结果，也能进行只读更新检查，但第三方工具与
包管理器仍是版本和升级的权威。内置步骤可以安装 mise 并把它接入受管 Shell profile，但不会替
mise 配置自身，也不会接管 runtime 版本。

### 让终端工作可重复

安装 `setproxy`、`copyfile` 等可迁移命令，保存按 argv 执行的个人任务，同步终端明暗主题，
并提供受管应用配置所需的本地生成资源。

### 在 SSH 两端延续工作

`shine ssh` 可以显式转发选定的环境值、延续终端主题，并通过已认证会话传输文件或目录。
远端只发起请求，由本机代理实际执行传输，无需另行暴露文件传输服务。

### 有边界地释放密钥

使用 GPG 或 age 保存工作区密钥，也可选用 macOS Touch ID。面对远程 AI 或工具工作流时，
SSH secret broker 始终在本地解密，并按精确的工作区、主机、命令和密钥策略匹配请求。

## 预设既是产品，也是扩展点

内置预设提供可直接使用的默认体验；其中 Surge 与 Clash Verge Rev 等 provider-specific
artifact 会承担应用特有的组装工作，减少用户心智负担。也可以把它们当作起点：复制一个类别、
用 overlay 覆盖少量路径，或维护完整的外部预设来源。命令输出会标明实际基础来源与 overlay。

生命周期命令以 `app/<category>`、`shell/<category>` 和 `sys/<item>` 为操作单元。Shell 命令
也可以通过 `shell/<category>/<command>` 单独安装或卸载。App 文件、脚本、driver 与 receipt
仍可通过 `info`、状态详情和 `--diff` 查看；App 文件不是独立安装单元，Shell 升级仍在类别
边界内协调已安装命令。

## 从这里开始

1. [安装 Shine](./installation.md)
2. [完成第一次预设安装](./quick-start.md)
3. 先查看[内置预设](./reference/built-in-presets.md)，确认类别、平台、目标路径和前提
4. 根据目标阅读 [Shell 预设](./guides/shell-presets.md)、[应用预设](./guides/app-presets.md)、[系统初始化](./guides/system-init.md)、[终端主题同步](./guides/terminal-theme-sync.md)、[任务与本地服务](./guides/tasks-and-serve.md) 或 [SSH 会话：密钥代理与文件传输](./guides/ssh-transfer.md)

如果已经遇到问题，直接前往[故障排查](./troubleshooting.md)。完整命令入口见[命令参考](./reference/commands.md)。
