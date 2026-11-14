---
title: 内置预设
sidebar_position: 3
---

# 内置预设

Shine 内置以下预设，可直接使用，也可复制后修改。它们并不代表
Shine 只能支持表中的工具。Shell 与 App 预设管理声明的命令和配置；系统预设负责初始化选定的
开发工具，或管理少量可逆的系统资源。App 预设不会自动安装对应的桌面应用。执行前先查看详情与
预览，确认目标路径、备份和权限范围：

```bash
shine shell list
shine app list
shine app info ghostty
shine app install ghostty --dry-run
shine sys list --all
shine sys info split-dns
```

本文介绍当前开发版（Next）的内置预设。已安装版本的可用功能请以 `shine list --available` 和 `--help` 的输出为准。

下表列出各平台可用的内置预设。

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

## Shell 预设

安装 Shell 类别后，打开新的 Bash、Zsh 或 PowerShell，即可按下表的名称调用命令。
需要修改当前会话的命令也应这样调用，不要直接运行预设目录中的脚本文件。

<div className="built-in-presets-shell-table" aria-hidden="true" />

| 类别 | 命令 | 用途与前提 |
| --- | --- | --- |
| `agent` | `ccenv` | 通过 Bun 在 macOS、Linux 或 Windows 选择 Codex（默认）、DeepSeek 或 Qwen，并启动 Claude Code。provider 变量只进入该子进程；需要已安装 `bun`、`shine` 和 Claude Code。 |
| `image-tools` | `img-compress` | 批量压缩直接传入的 JPEG、PNG、WebP 文件或目录第一层图片，生成派生文件。使用 `IMAGE_QUALITY`；需要 Bun 1.3.14 或更高版本。 |
| `image-tools` | `img-resize` | 批量缩放图片，使其不放大并限制在 `IMAGE_MAX_WIDTH` × `IMAGE_MAX_HEIGHT` 内。需要 Bun 1.3.14 或更高版本。 |
| `image-tools` | `img-convert` | 使用明确的 `--format` 把图片批量转换为 JPEG、PNG 或 WebP。需要 Bun 1.3.14 或更高版本。 |
| `proxy` | `setproxy` | 为当前会话设置 HTTP/HTTPS/SOCKS5、npm 和 pnpm 代理。使用 `HTTP_PROXY_PORT`、`SOCKS5_PROXY_PORT`、`PROXY_HOST`、`PROXY_NO_PROXY`；`auto` 优先 SOCKS5。若安装了 Yarn，它会修改 Yarn 的持久代理配置。 |
| `proxy` | `usetproxy` | 清除当前会话的代理变量，并清除 `setproxy` 写入的 Yarn 代理配置。 |
| `utils` | `copyfile` | 通过 OSC52 把一个文件内容复制到本地剪贴板；仅 Unix，终端或终端复用器必须允许 OSC52。 |
| `utils` | `shine-env-export` | 将一个 Shine env 值导入当前会话；可读取同名明文值，或解密 `<KEY>_SECRET`。 |
| `utils` | `shine-theme-sync` | 为当前会话设置终端明暗主题对应的 `SHINE_TERMINAL_THEME` 与 `BAT_THEME`。 |

`ccenv` 的 Codex provider 通过本机 CLIProxyAPI 使用 `CLIPROXYAPI_AUTH_TOKEN`；DeepSeek 和 Qwen 分别使用对应的 `*_API_KEY`。凭据按 `_SECRET`、旧版 `_GPG_SECRET`、明文值的顺序解析；一旦选中的密文解密失败就会停止，不会回退。CLIProxyAPI 应只绑定回环地址，并配置与客户端相同的 token。

更多安装、重装与卸载说明见[管理 Shell 预设](../guides/shell-presets.md)，环境变量格式见[管理环境变量](../guides/environment.md)。

### 图片工作流 {#image-workflow}

这些命令压缩、缩放和转换 JPEG、PNG、WebP。运行命令的每台机器都必须在 `PATH` 中安装
Bun 1.3.14 或更高版本；Shine 不内置 Bun，无需另外安装图片处理库。

下面的流程会使用三个命令，因此先安装整个类别：

```bash
shine info shell/image-tools
shine install shell/image-tools
```

安装后，按 [Shell 预设](../guides/shell-presets.md) 的说明打开新终端或重新加载 shell 配置，再运行：

```bash
img-compress photo.jpg screenshots/
img-resize --width 1280 --output-dir ./resized photos/
img-convert --format webp --quality 75 --output-dir ./webp hero.png gallery/
shine info shell/image-tools --diff
shine upgrade shell/image-tools
shine shell uninstall image-tools/img-compress --dry-run
```

最后一条命令只预览移除 `img-compress`。如果一开始就只需要这个命令，可以安装
`shell/image-tools/img-compress`，无需安装整个类别。

每个命令都接受多个文件或目录输入。目录只扫描第一层的 JPEG、PNG、WebP，不会递归。未指定
`--output-dir` 时，结果保存在来源旁边，名称形如 `photo.compressed.jpg`、
`photo.resized.jpg` 或所选转换格式的扩展名。指定输出目录后，所有结果平铺到该目录；重复目标名会
明确失败。已有目标也会失败，只有 `--force` 才允许替换；来源图片永远不会被原地修改。
`--force` 也不允许替换输入图片。

批处理遇到单项失败后会继续，并在存在任意失败时返回非零状态。前 20 条失败会显示在终端；超过
20 条时，完整列表还会写入 `--output-dir` 下唯一命名的 `image-tools-errors-*.log`，未指定输出目录
时则写入当前目录。

`IMAGE_QUALITY`、`IMAGE_MAX_WIDTH`、`IMAGE_MAX_HEIGHT` 默认分别为 `80`、`1920`、`1080`。
命令参数只覆盖当次运行，本地 Shine 配置则会保留这台机器的长期偏好。默认 snapshot 模式下，修改
复制或外部来源后仍需执行 `shine upgrade`，已安装命令才会变化。

## App 预设

大多数预设安装配置文件。Surge 和 Clash Verge Rev 还提供显式 artifact 命令，将这些文件接入
应用当前配置；请按各自指南完成设置。

<div className="built-in-presets-app-table" aria-hidden="true" />

| 类别 | 平台与目标 | 管理内容与注意事项 |
| --- | --- | --- |
| `archey4` | Unix；`~/.config/archey4/` | Archey4 系统信息配置。 |
| `clash-verge` | macOS、Linux、Windows；merge 源位于 `~/.shine/clash-verge/`，本地参考规则位于 CVR 用户数据目录 | Clash Verge Rev 的订阅增强示例。模式 1 使用受管 HomeDir 规则文件，HTTP/HTTPS 模式不会引用它们。构建与清理需要 Bun；实际订阅编辑器绑定与完整流程见[应用配置指南](../guides/app-presets.md#clash-verge-rev)。 |
| `docker-desktop` | Windows；`~/AppData/Roaming/Docker/settings-store.json` | 仅修改 `proxy` 与 `containersProxy` 设置，完成后重启 Docker Desktop。 |
| `docker-engine` | Unix：`/etc/docker/daemon.json`；Windows：`~/.docker/daemon.json` | Docker daemon 配置；Unix 目标需要管理员权限，完成后重启 Docker Engine。 |
| `fastfetch` | `~/.config/fastfetch/` | Fastfetch 系统信息配置。 |
| `ghostty` | Unix；`~/.config/ghostty/` | Ghostty 主配置与内置明暗主题；主题背景可使用 `GHOSTTY_BG_LIGHT`、`GHOSTTY_BG_DARK` 模板变量。Ctrl + `\` 向右新建分屏，Ctrl + `-` 向下新建分屏。Ctrl + H/J/K/L 分别切换到左/下/上/右分屏；解除 Super + [ / ] 的绑定（macOS 上为 Cmd）。无法切换分屏时，按键交给终端内的程序处理。 |
| `git` | `~/.gitconfig` | Git 常用别名和默认配置。安装前会按普通 app 预设规则备份不受管文件。 |
| `JetBrains` | `~/.ideavimrc` | JetBrains 的 IdeaVim 配置；需要已在 IDE 中启用 IdeaVim 插件。普通模式下，Ctrl + H/J/K/L 分别切换到左/下/上/右编辑器分屏，与 Ghostty 预设保持一致。 |
| `starship` | `~/.config/starship.toml` | Starship prompt 配置；需要另行安装并在 shell 中启用 Starship。 |
| `surge` | macOS；`~/Library/Application Support/Surge/Profiles/` | 本地代理、策略组、规则、General 设置模块，以及可选的 URI 订阅生成文件。Artifact apply/remove 需要 Bun，以及正在运行且 CLI 支持模块管理的 Surge；生成、刷新和 artifact 流程见[应用配置指南](../guides/app-presets.md#生成式文件与-surge-uri-订阅)。 |
| `vim` | `~/.vim/` | Vim 基础配置和机器本地覆盖文件。 |

`docker-desktop` 的 JSON 合并保留其它 Docker Desktop 设置；所有其它 app 预设只管理其各自声明的文件。安装应用预设不会下载、安装或启动 Ghostty、Docker、Surge、Starship 等应用。

## 系统预设

使用 `shine sys bootstrap` 安装选定的开发工具，或通过 `--preset` 选择下表中的配置组合。
执行前先用 `--dry-run` 检查步骤和所需权限；完整操作见[初始化与管理系统](../guides/system-init.md)。

安装后会启用所选工具的 Shell 集成，之前启用的集成也会保留；如需关闭，使用
`shine sys profile disable <ITEM>`。Shine 不升级已安装的软件，请使用对应的包管理器或上游工具。
`mise` 项目只安装并启用 mise；工具配置和版本管理仍由 mise 自己负责。

<div className="built-in-presets-system-table" aria-hidden="true" />

| 平台 | Profile | 包含项目 |
| --- | --- | --- |
| macOS | `required` | Homebrew、Yazi、Starship。 |
| macOS | `recommended`（默认） | `required` 加 Rust、Neovim、AstroNvim、ZeroTier、Zsh 插件、zoxide、Atuin、fzf、bat、eza。 |
| macOS | `all` | `recommended` 加 nvm、Bun、pnpm、mise、Fastfetch。 |
| Ubuntu | `recommended`（默认） | Rust、Neovim、AstroNvim、Atuin、Yazi、Starship、zoxide、zsh-vi-mode、fzf、bat、eza。 |
| Ubuntu | `all` | `recommended` 加 ZeroTier、Bun、pnpm、mise、Homebrew。 |
| Ubuntu | `minimal` | Neovim、fzf、bat、eza、zoxide；适合不需要 shell 历史、prompt 与 JavaScript 工具链的服务器。 |
| Arch Linux | `minimal`（默认） | 通过 pacman 安装 Neovim、fzf、bat、eza、zoxide，支持 Bash/Zsh 集成。 |
| Windows | `required` | Rust、Yazi、Starship。 |
| Windows | `recommended`（默认） | `required` 加 Neovim、zoxide、Atuin、fzf、bat、eza、ZeroTier。 |
| Windows | `all` | `recommended` 加 Bun、pnpm、mise。 |

Arch 的安装前提与全系统更新要求见 [Arch Linux 准备工作](../guides/system-init.md#arch-linux-preparation)。

### Ubuntu 安装说明 {#ubuntu-installation-notes}

- **Atuin：** 安装后打开新的 Shell，运行 `atuin import auto` 导入已有历史；需要同步历史时，
  再运行 `atuin register` 或 `atuin login`。
- **Yazi：** 从官方 stable APT 源安装软件及预览所需依赖，需要管理员权限，无需预先安装
  Rust 或 Cargo。已有安装会被跳过，不会自动更换安装方式。
- **pnpm：** 无需预先安装 Node.js、npm 或 Corepack。安装目录为 `~/.local/share/pnpm`，
  Shine 会配置命令路径；安装后打开新的 Shell 即可使用。
- **最小服务器配置：** `minimal` 仅包含上表所列工具，不含历史同步、提示符、Node.js 工具链或
  Homebrew。安装前运行 `shine sys bootstrap --preset minimal --dry-run` 确认实际步骤。

### Private split DNS

macOS、Ubuntu 和 Windows 内置独立的受管 `split-dns` 项目，不属于上述默认 profile。它把一个私有域名后缀定向到可经 ZeroTier 访问的 DNS 服务器；需要管理员权限和以下生效的 env 值：

```toml
PRIVATE_DNS_DOMAIN = "home.example.internal"
PRIVATE_DNS_SERVERS = "10.0.0.53"
```

先预览，再应用或移除：

```bash
shine sys info split-dns
shine sys apply split-dns --dry-run
shine sys apply split-dns
shine sys uninstall split-dns --dry-run
```

Ubuntu 上，`split-dns` 要求本机 DNS 查询经过 `systemd-resolved` 的 `127.0.0.53`。
如果 Shine 提示 DNS stub 被关闭，请先重新启用 `DNSStubListener`，或确认本机解析链路会经过
`systemd-resolved`，再应用配置。

它不会安装 ZeroTier、CoreDNS 或创建 DNS 区域；应先确保私有网络和 DNS 服务可用。完整的跨平台网络与排错步骤见[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns)。

系统初始化、受管项目和代理使用方式见[初始化与管理系统](../guides/system-init.md)。
