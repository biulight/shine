---
title: 安装与升级
sidebar_position: 2
---

# 安装与升级

Shine 支持 macOS、Linux 和 Windows。官方安装脚本从 GitHub Releases 下载当前平台的二进制文件。

## macOS 与 Linux

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

默认安装到 `~/.local/bin/shine`，安装脚本不会修改 shell 配置。确认 `~/.local/bin` 已在 `PATH` 中，然后验证：

```bash
shine --version
```

如需指定位置或版本：

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=1.2.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

默认安装到 `%LOCALAPPDATA%\Programs\shine\shine.exe`，不会修改用户 `PATH`。如需指定位置或版本：

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "1.2.0"; .\install.ps1
```

## 从源码安装

已安装 Rust **1.88 或更高版本**时，可从 crates.io 安装：

```bash
cargo install shine-cli
```

若在 Shine 源码仓库中构建，运行 `cargo build --release`；二进制文件位于
`target/release/shine`。

## 升级 Shine

安装后可由 Shine 下载稳定版或 preview 版：

```bash
shine self upgrade
shine self upgrade --channel stable
shine self upgrade --channel preview
```

`preview` 是持续滚动的预发布通道，不参与日常自动更新检查。运行 `shine update` 可以同时检查已安装配置和稳定版程序更新。

在 Unix 系统上，如果 `shine self install` 要把二进制文件复制到当前用户不可写的位置，Shine 会交互式请求授权，并通过 `sudo` 完成安装。`shine self upgrade` 成功后同步另一个已记录的安装目标时，也会采用相同方式。不过，如果当前运行的 Shine 本身位于受保护目录，Shine 暂时无法自动提权替换该二进制文件；请改为安装并运行用户可写位置中的 Shine。Windows 在受保护位置安装或升级时，仍需使用具备相应权限的终端。

下一步：[完成第一次预设安装](./quick-start.md)。
