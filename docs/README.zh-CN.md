# Shine

**Shine 帮你统一管理常用脚本工具和应用配置，让敏感信息加密保存、按需使用。**

Shine 是命令行工具。你可以直接使用已有的 **Preset（预设）**，无需自己编写脚本，也可以整理自己的
脚本与配置，方便复用。所需的应用和运行环境需另行安装。

**完整文档：**[简体中文](https://biulight.github.io/shine/zh-Hans/) ·
[English](https://biulight.github.io/shine/)

[![Shine 功能概览：管理常用脚本工具与应用配置，加密保存敏感信息并按需交给程序，减少 AI 辅助开发时项目中的明文密钥。](../website/static/img/shine-overview-zh-Hans.webp)](../website/static/img/shine-overview-zh-Hans.webp)

## Shine 能替你做什么

- **复用脚本与配置：** 在预设中集中维护，按需安装，每台电脑专用的参数留在本地。
- **变更前先审阅：** 更新预设前先查看变化；卸载时默认保留被修改或无法确认由 Shine 管理的文件，交给你检查。
- **敏感信息按需使用：** 将密码、访问密钥和敏感文件加密保存，在运行指定程序时提供。详见
  [环境变量与密钥](https://biulight.github.io/shine/zh-Hans/guides/environment)。
- **减少 AI 辅助开发中的明文密钥：** 完成加密并处理原有明文后，降低 AI 扫描、复制或提交项目文件时
  意外暴露密钥的风险。详见[使用 AI 工具时保护密钥](https://biulight.github.io/shine/zh-Hans/guides/agent-secret-safety)。

获得密钥或具备解密能力的程序仍能读取明文。临时文件清理可能失败，异常中断后应检查残留。

## 现在就能试

- 安装 `shell/proxy`，把 `setproxy` 和 `usetproxy` 作为普通命令使用。
- 查看 Starship、Git、Vim、Ghostty 等工具已有的配置预设。
- 需要配置 Surge 或 Clash Verge Rev 时，使用已经准备好的专用流程。

```bash
shine list --available
shine install shell/proxy
shine info shell/proxy
```

完整的第一次使用流程见[快速开始](https://biulight.github.io/shine/zh-Hans/quick-start)。

## 也可以自己做

预设文件夹可以通过任意文件夹同步工具、压缩包、网络传输、版本管理工作区或手工复制到达机器，
Shine 不限定你怎样分享它。

你可以把批量重命名、表格整理或文档打印等重复操作封装成自己的可复用命令。每个命令需要的应用或
运行环境仍要安装在对应机器上。可以从[自定义预设指南](https://biulight.github.io/shine/zh-Hans/guides/custom-presets)开始。

Shine 也能初始化 macOS、Ubuntu 或 Windows 的选定部分，但不会接管第三方工具版本；具体边界见
[系统初始化](https://biulight.github.io/shine/zh-Hans/guides/system-init)。

## 安装

macOS 与 Linux：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

已安装 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 时：

```bash
cargo binstall shine-cli
```

已安装 Rust 1.90 或更高版本时，可从 crates.io 编译安装：

```bash
cargo install shine-cli
```

支持的预编译平台及安装选项见[安装与升级](https://biulight.github.io/shine/zh-Hans/next/installation)。

开发、测试、规划与发布流程以仓库根目录的 [`README.md`](../README.md)、[`AGENTS.md`](../AGENTS.md)、
[`docs/PLAN.md`](PLAN.md) 和 [`docs/kb/`](kb/) 为准。

## 许可证

MIT OR Apache-2.0
