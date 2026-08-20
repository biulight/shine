# Shine

Shine 是一个跨平台 Rust CLI，用于在不同机器与远程会话之间保持开发环境可迁移、可用且安全。

Shine 把 Shell 命令、应用配置、引导步骤、环境变量和远程会话工作流变成归属明确的资源。它会
跟踪自己管理的内容，让用户先检查再应用变更，并在不接管无关用户文件的前提下安全移除。

**完整文档：**[简体中文](https://biulight.github.io/shine/zh-Hans/) ·
[English](https://biulight.github.io/shine/)

## Shine 连接的工作流

- **跨机器初始化：**安装并持续对齐 Shell 与 App 预设，再用边界明确的系统脚本初始化新的
  macOS、Ubuntu 或 Windows 环境。
- **可重复终端工作：**保存任务、安装可迁移的辅助命令、同步终端主题，并提供生成式本地资源。
- **本地与远端连续性：**通过 `shine ssh` 显式转发选定变量，并在已认证会话中传输文件。
- **有边界的密钥访问：**使用 GPG 或 age 加密 workspace 值，或让远程 AI/工具工作流按本地
  精确策略请求密钥。

内置预设既是可直接使用的默认体验，也是定制起点。Surge 与 Clash Verge Rev 的专用 artifact
减少 provider-specific 配置负担；`preset copy`、overlay 与外部 Git 来源则用于局部或完整定制。

`shine sys` 的范围比 App 与 Shell 生命周期更窄：内置脚本负责初始化，少量 driver 管理
split DNS 等可逆系统资源，但不接管第三方 runtime 版本。例如 sys 预设可以安装并激活 mise，
mise 的配置和工具版本仍由 mise 自己管理。

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

生命周期操作使用 `app/starship`、`shell/proxy` 和 `sys/split-dns` 这样的类别级规范 target；
文件与命令身份属于检查详情。完整流程见
[快速开始](https://biulight.github.io/shine/zh-Hans/quick-start)与
[命令参考](https://biulight.github.io/shine/zh-Hans/reference/commands)。

开发、测试、规划与发布流程以仓库根目录的 [`README.md`](../README.md)、[`AGENTS.md`](../AGENTS.md)、
[`docs/PLAN.md`](PLAN.md) 和 [`docs/kb/`](kb/) 为准。

## 许可证

MIT OR Apache-2.0
