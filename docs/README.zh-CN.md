# Shine

Shine 是一个跨平台 Rust CLI，用于管理 shell 命令、应用配置、系统资源、分层环境变量和可重复的
机器初始化流程。

Shine 将常用预设打包在一个自包含二进制中，并通过 manifest 跟踪受管文件，使其可以被检查、
更新和安全移除，而不会接管无关的用户内容。也可以使用外部预设仓库和按路径覆盖的 overlay。

**完整文档：**[简体中文](https://biulight.github.io/shine/zh-Hans/) ·
[English](https://biulight.github.io/shine/)

## 主要能力

- 安装可移植的 shell 命令和应用配置，支持 dry-run 与用户修改保护。
- 在 macOS、Ubuntu 和 Windows 上初始化开发环境。
- 管理分层环境变量，并通过 GPG 或 age 加密 workspace 密钥。
- 保存个人任务、同步终端主题，并提供受管的本地资源。
- 通过 `shine ssh` 转发选定变量、按需代理本机密钥和传输文件。
- 使用外部来源或局部 overlay 扩展内置预设。

## 安装

macOS 与 Linux：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

已经安装 Rust 1.88 或更高版本时：

```bash
cargo install shine-cli
```

## 快速示例

```bash
shine list --available
shine info shell/proxy
shine install shell/proxy
shine update
shine upgrade shell/proxy
```

资源使用 `app/starship`、`shell/proxy` 和 `sys/split-dns` 这样的规范 target。完整流程见
[快速开始](https://biulight.github.io/shine/zh-Hans/quick-start)与
[命令参考](https://biulight.github.io/shine/zh-Hans/reference/commands)。

开发、测试、规划与发布流程以仓库根目录的 [`README.md`](../README.md)、[`AGENTS.md`](../AGENTS.md)、
[`docs/PLAN.md`](PLAN.md) 和 [`docs/kb/`](kb/) 为准。

## 许可证

MIT OR Apache-2.0
