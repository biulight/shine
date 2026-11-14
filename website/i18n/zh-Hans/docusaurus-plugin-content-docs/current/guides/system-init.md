---
title: 初始化与管理系统
sidebar_position: 3
---

# 初始化与管理系统

使用系统预设，可以在 macOS、Ubuntu、Arch Linux 或 Windows 上安装选定的开发工具，并启用相应的 Shell
集成。Shine 也能单独管理私有 DNS 等系统设置，但不负责升级第三方软件。

## 先查看，再预览 {#先查看再执行}

列出当前系统可用的项目，并查看安装要求：

```bash
shine sys list
shine sys info neovim
```

使用 `shine sys list --all` 可浏览其他平台的项目。各平台的项目与配置组合（profile）见
[内置预设](../reference/built-in-presets.md#系统预设)。

预览想要安装的项目：

```bash
shine sys bootstrap neovim fzf --dry-run
```

预览会显示安装步骤和 Shell 配置改动，不会实际执行。继续之前，请确认所需的管理员权限、
下载操作和环境变量。

## 选择并安装工具 {#交互选择或应用-profile}

按需选择一种方式：

| 需求 | 命令 |
| --- | --- |
| 交互选择项目 | `shine sys bootstrap` |
| 只安装 fzf | `shine sys bootstrap fzf` |
| 依次安装 Neovim 和 fzf | `shine sys bootstrap neovim fzf` |
| 使用推荐配置组合（macOS、Ubuntu、Windows） | `shine sys bootstrap --preset recommended` |
| 使用 Ubuntu 或 Arch Linux 的最小配置组合 | `shine sys bootstrap --preset minimal` |

实际修改系统前，Shine 会显示操作计划并请求确认，默认为否。请检查改动、所需权限和恢复限制；
需要更多详情时添加 `--verbose`。自定义脚本会以你的权限运行，没有沙箱隔离；请先审阅外部代码，
再批准执行。

安装期间按提示操作，并等待本次操作结束。项目按顺序执行，遇到首个失败或超时便停止，
每个安装器最多运行 30 分钟。是否成功请以 Shine 的最终结果为准，不要仅凭安装日志中的成功提示判断。

在脚本中使用时，请参阅[系统命令选项](../reference/commands.md#系统预设)，了解 `--yes`
及外部代码的授权要求。

## Arch Linux 准备工作 {#arch-linux-preparation}

仅支持官方 x86_64 Arch Linux，目前只提供默认的 `minimal` 预设，
包含 Neovim、fzf、bat、eza 和 zoxide。

安装前，请先查阅 Arch 的更新说明并完成全系统更新（root 用户省略 `sudo`）。
确认更新成功后，再预览并安装：

```bash
sudo pacman -Syu

shine sys bootstrap --preset minimal --dry-run
shine sys bootstrap --preset minimal
```

Shine 不会自动升级系统。Arch 不支持部分升级，请勿单独运行 `pacman -Sy` 后安装软件。

下载失败并出现 HTTP 404 时，请确认系统更新已成功完成；仍有问题时检查所配置的镜像。
其他错误请根据 pacman 输出排查。参阅 [Arch 系统维护说明](https://wiki.archlinux.org/title/System_maintenance)。

## 打开新终端并查看结果

安装完成后，打开新的 Shell，让新命令和集成生效。查看已保存的结果：

```bash
shine sys status
```

这里展示每个项目最近一次保存的结果，可能来自不同次运行；它不会实时检查软件是否仍然存在，
或系统配置是否与记录一致。

Atuin 历史导入等软件专属步骤见[Ubuntu 安装说明](../reference/built-in-presets.md#ubuntu-installation-notes)。

## 下载时使用代理

添加 `--proxy` 可使用 Shine 的 HTTP 代理设置。先预览，再安装：

```bash
shine sys bootstrap fzf --proxy --dry-run
shine sys bootstrap fzf --proxy
```

在 Shine 的[环境设置](./environment.md)中配置 `PROXY_HOST`、`HTTP_PROXY_PORT` 和
`PROXY_NO_PROXY`。默认代理地址为 `http://127.0.0.1:6152`，请在预览中确认实际值。

Windows 上，winget 可能需要先在管理员 PowerShell 中启用代理选项：

```powershell
winget settings --enable ProxyCommandLineOptions
```

## 启用或禁用 Shell 集成 {#管理-shell-集成状态}

Shell 集成用于让终端找到工具，或启用提示符、历史搜索等功能。安装所选项目时会启用对应集成，
之前启用的集成仍然保留；改选其他配置组合也不会关闭它们。

| 需求 | 命令 |
| --- | --- |
| 预览禁用 fzf 集成 | `shine sys profile disable fzf --dry-run` |
| 禁用集成 | `shine sys profile disable fzf` |
| 重新启用 | `shine sys profile enable fzf` |

这些命令只修改 Shine 管理的 Shell 配置。禁用不会卸载软件；启用前需要先安装软件。
`shine upgrade` 不会启用或禁用这些集成。修改后请打开新的 Shell。

如果结果显示 `needs action`（需要处理），请检查输出指出的文件。合并需要人工处理时，Shine
会保留当前文件；即使命令正常退出，也应确认请求的集成是否已经生效。

## 升级已安装的软件 {#用软件自己的工具升级}

请使用安装该软件的包管理器或上游工具，例如 Homebrew、apt、pacman、winget、mise 或 rustup。
重新运行 `shine sys bootstrap` 会跳过已检测到的软件，不会升级它。`shine update` 和
`shine upgrade` 处理的是 Shine 管理的配置，不负责第三方软件版本。

使用 mise 时，工具配置、安装与版本管理仍由 mise 自己负责。

## 管理系统设置 {#受管系统项目}

私有 DNS 等受管配置使用 `apply` 和 `uninstall` 管理。使用 `split-dns` 前，请先完成
[私有 DNS 准备步骤](../reference/built-in-presets.md#private-split-dns)。

| 需求 | 命令 |
| --- | --- |
| 预览私有 DNS 改动 | `shine sys apply split-dns --dry-run` |
| 应用配置 | `shine sys apply split-dns` |
| 预览移除 | `shine sys uninstall split-dns --dry-run` |
| 移除配置 | `shine sys uninstall split-dns` |

执行前请审阅计划并确认；需要管理员权限时，Shine 会在确认后请求。使用
`shine sys info split-dns` 查看详情，使用 `shine sys status` 查看已保存的结果。

## 处理失败与冲突

- **安装失败或超时：** 查看报错项目，按安装器或包管理器的提示修复。`shine sys recover`
  不能撤销软件安装。
- **受管操作中断：** Shine 提示需要恢复时，运行 `shine sys recover` 并审阅计划。只有相关状态
  未发生变化时，才会完成或回退原操作；修改过的资源和无关的 Shell 内容会保留。
- **另一项系统操作仍在运行：** 等待它结束。如果 Shine 提示状态发生变化，请重新审阅计划后重试。
- **Shell 配置冲突：** 检查 Shine 指出的文件。只有希望备份并替换冲突配置，而非保留和合并自己的
  修改时，才使用 `shine sys bootstrap --force-profile`。
- **旧版 Ubuntu 配置中找不到 Rust 命令：** 升级 Shine，运行 `shine sys bootstrap rust`，
  然后打开新的 Shell。这会修复集成，不会重装已检测到的 Rust。Bash/Zsh 中若存在
  `$HOME/.cargo/env`，也可运行 `. "$HOME/.cargo/env"` 使当前会话生效。

## 支持的 Shell {#平台说明}

macOS 支持 zsh，Ubuntu 和 Arch Linux 支持 bash 和 zsh，Windows 使用 PowerShell。
Ubuntu、Arch Linux 和 macOS 的终端外观自动设置见[终端主题同步](./terminal-theme-sync.md)。

## 清理旧版本的本地 HTTP 服务 {#retire-legacy-http-service}

如果曾使用 Shine 的旧 HTTP 服务，请按[升级与迁移中的清理步骤](./upgrading.md#清理旧版本的本地-http-服务)处理。
