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

默认安装到 `~/.local/bin/shine`，安装脚本不会修改 Shell 配置。
如果提示安装目录不在 `PATH` 中，请先执行输出中的配置命令，再执行使当前终端生效的命令。
如果配置文件无法更新，请根据错误提示手动配置。
如果使用的 Shell 与提示不同，或使用 Fish 等其他 Shell，请手动将该目录加入相应的配置。
随后验证：

```bash
shine --version
```

如需指定位置或版本，先下载脚本，再选择一种方式运行：

```bash
curl -fsSLo install.sh https://github.com/biulight/shine/releases/latest/download/install.sh
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=2.10.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

默认安装到 `%LOCALAPPDATA%\Programs\shine\shine.exe`，不会修改用户 `PATH`。
如果提示找不到安装目录，请按输出指引将其加入用户 `PATH`，打开新终端并运行 `shine --version`。

如需指定位置或版本，先下载脚本，再选择一种方式运行：

```powershell
Invoke-WebRequest https://github.com/biulight/shine/releases/latest/download/install.ps1 -OutFile install.ps1
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "2.10.0"; .\install.ps1
```

## 安装 preview

这些命令使用与正式版相同的安装位置，并替换该位置已有的 Shine 二进制文件。
两个平台的 preview 安装脚本均默认安装预览版；如果已设置 `SHINE_VERSION` 环境变量，则以该变量为准。preview 是预发布版本，稳定性可能低于正式版。安装后运行
`shine --version` 检查版本。

在 macOS 或 Linux 上直接安装滚动更新的 preview 版本：

```bash
curl -fsSL https://github.com/biulight/shine/releases/download/preview/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://github.com/biulight/shine/releases/download/preview/install.ps1 | iex
```

使用 `shine self upgrade --channel preview` 更新预览版，或通过
`shine self upgrade --channel stable` 切回最新正式版。安装 preview 不会保存更新通道设置：
不指定通道的 `shine self upgrade` 仍选择正式版。

## 使用 cargo-binstall 安装

已安装 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 时，可从官方 GitHub Releases
下载预编译的 Shine 二进制文件：

```bash
cargo binstall shine-cli
shine --version
```

包名是 `shine-cli`，安装后的命令是 `shine`。默认安装到 `$CARGO_HOME/bin`
（通常为 `~/.cargo/bin`），请确认该目录已加入 `PATH`。

官方二进制文件覆盖 macOS、使用 glibc 的 Linux 和 Windows（MSVC 构建），均提供 x86_64 和
aarch64 两种架构。Shine 目前未发布 Linux musl 预编译包。

默认情况下，找不到适用的官方二进制文件时，cargo-binstall 还会查询外部的
[cargo-quickinstall](https://github.com/cargo-bins/cargo-quickinstall) 构建服务，最后回退到从源码编译。
如需仅使用官方预编译二进制文件，并在没有适用文件时报错退出，使用：

```bash
cargo binstall shine-cli --strategies crate-meta-data
```

再次运行 `cargo binstall shine-cli` 可通过 cargo-binstall 升级。滚动的 `preview` 通道请使用
`shine self upgrade --channel preview`。

## 从源码安装

已安装 Rust **1.90 或更高版本**时，可从 crates.io 安装：

```bash
cargo install shine-cli
```

若在 Shine 源码仓库中构建，运行 `cargo build --release`；二进制文件位于
`target/release/shine`。

## 升级 Shine

按需选择一条更新命令：

| 操作 | 命令 |
| --- | --- |
| 升级到最新稳定版 | `shine self upgrade` |
| 重新安装最新稳定版，也可从 preview 切回 | `shine self upgrade --channel stable` |
| 更新到滚动预览版 | `shine self upgrade --channel preview` |

`--channel stable` 即使在当前版本相同或更新时也会执行重新安装。

`preview` 是持续滚动的预发布通道，不参与日常自动更新检查。
运行 `shine update` 可以同时检查已安装配置和稳定版程序更新。

<span id="升级到-shine-28" />
<span id="升级到-shine-272" />
<span id="升级到-shine-271" />
<span id="升级到-shine-27" />
<span id="升级到-shine-26" />
<span id="清理旧版本的本地-http-服务" />

升级前请阅读[升级与迁移](./guides/upgrading.md)，按当前版本和使用的功能确认必要操作。
新增功能与修复列表见[版本说明](https://github.com/biulight/shine/releases)。

使用 `shine self install --dest <PATH>` 可将 Shine 复制到指定路径。相对路径以当前目录为起点；
请将目标文件所在目录加入 `PATH`。后续自升级也会更新该副本。如果提示副本更新失败，它可能仍是
旧版本；请解决提示中的问题，再用升级后的 Shine 重新执行该安装命令。

Unix 上，安装或同步到受保护位置时会请求 `sudo` 授权。但从受保护目录运行的 Shine 无法自行
提权升级；如需使用自升级，请安装并运行用户可写目录中的 Shine。Windows 在受保护位置安装
或升级时，需使用具备相应权限的终端。

## 卸载 Shine

在需要清理的账户和配置下运行 `shine self uninstall --dry-run`，核对列出的变更。
运行 `shine self uninstall` 可交互确认，运行 `shine self uninstall --yes` 可直接批准。

Shine 按各自的卸载规则处理受管 App 文件、Shell 配置、Sys 项和环境代理。对于账户默认配置，
还会检查 Sys 启动文件中与原文一致的加载块，以及旧版 HTTP 服务。个人设置、任务、外部预设、
加密身份、自定义 Sys 配置和 bootstrap 安装的软件会保留。配置目录中仍可能有这些文件、
缓存和锁文件。预览会列出准备移除的信任授权和 broker 策略文件。
如果 Shine 无法查询账户的服务管理器，请切换到能查询它的会话后重试。如果使用自定义配置
目录时检测到旧服务，请单独核查 `shine clear serve` 的清理范围。

如果未完成的操作或被修改的受管文件阻止清理，请按提示处理对应类型的恢复或卸载，再运行本命令。
如果 Shell 启动块指向另一套 Shine 配置或含有手动修改，请核查提示中的文件。只有与当前配置生成内容一致的启动块才会自动移除。
此前已成功的步骤继续生效。Shine 最后才移除能够确认由它安装且未被修改的程序副本。已更换或版本不同的副本需要单独核查。

如果通过 Cargo 安装，请使用 Shine 输出的 Cargo 卸载命令。手动下载或缺少匹配回执的旧安装，
请在受管资源清理完成后按原安装渠道移除程序。在 Windows 上，先关闭正在运行的实例，
再从同一构建的另一份副本、使用同一配置运行 `self uninstall`。完成清理后，发起命令的副本会保留。

下一步：[完成第一次预设安装](./quick-start.md)。
