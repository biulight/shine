# shine

一个用于管理 shell 预设、应用配置和系统初始化预设的 Rust CLI。

`shine` 将可复用的 shell 脚本、应用配置预设和操作系统初始化预设打包进一个二进制中。它会把受管资产安装到 `~/.shine/`，把 shell 命令链接到 `~/.shine/bin/`，也可以把应用配置文件复制到最终目标位置。

English README: [`../README.md`](../README.md)

## 功能特性

- **内置预设** — shell 脚本和应用配置会编译进二进制；安装后不需要联网
- **外部预设目录和 overlay** — 可用 `presets_dir` 完整替换预设来源，也可链接一个小型 overlay 目录，只覆盖少量内置预设文件
- **项目本地预设仓库** — 在预设仓库内运行 `shine init`，即可创建指向当前仓库的 `shine.config.toml`
- **受管 bin 目录** — `~/.shine/bin/` 在 Unix 上保存展平后的符号链接，在 Windows 上保存命令 shim
- **自动配置 PATH** — `install` 会自动把 `~/.shine/bin` 追加到你的 shell 配置文件
- **按类别安装/卸载** — 可安装或卸载全部预设，也可只处理某个子集（如 `proxy`）
- **仅显示已安装项** — `shine list` 只展示已安装内容，不输出额外状态噪音
- **安全卸载** — 只删除 `shine` 管理的文件；用户自行创建的文件不会被触碰
- **支持 dry-run** — 任何破坏性操作都可以先预览再执行
- **TOML 配置** — 使用 `~/.shine/config.toml`，更新时会尽量保留注释
- **应用预设安装器** — 可安装 `~/.gitconfig`、`~/.config/starship/starship.toml`、`~/.config/ghostty/config.ghostty` 等受管配置
- **已安装内容检查** — `shine info <target>` 会输出已安装应用配置和 shell 预设的元数据、彩色状态和值得关注的预期内容差异；加 `--verbose` 可查看完整内容
- **版本更新检查** — 运行时检查 GitHub Releases，并使用 24 小时缓存
- **多 shell 支持** — bash、zsh 和 PowerShell；当同一类别在 Unix 和 Windows 需要不同文件时，可按平台声明 shell 预设条目
- **系统初始化预设** — 通过 `shine sys init` 对当前操作系统执行一组整理过的初始化步骤

当前支持范围：`shine shell` 支持 `bash`、`zsh` 和 PowerShell。Windows 支持目前覆盖 `shine self`、`shine shell`，`docker-engine`、`docker-desktop` 这类已适配的 app 预设，以及用 PowerShell 实现的 Windows `shine sys init` 预设。

## 规划流程

仓库规划通过 GitHub 采用一套轻量的问题单流程管理：

- 使用 `Idea / Plan` issue 模板记录新想法
- 将已接受的工作提升为 `Task` issue
- 用 `status:` 标签跟踪状态
- 只对和发布有关的工作使用 milestone

完整规则见 [`PLAN.md`](PLAN.md)。

## 发布分支流程

- `release` 是主要的集成和发布分支。
- 日常提交和功能 PR 应以 `release` 为目标分支。
- 版本标签（`v*`）应从 `release` 创建；CI 会构建产物并发布 GitHub Release。
- CI 创建完 GitHub Release 后，会自动发起一个从 `release` 到 `main` 的 PR。
- `main` 只用于这个发布后的同步 PR，而不是日常开发。

## 安装

macOS/Linux：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

或从源码安装：

```bash
cargo install --path cli
```

Windows 支持目前覆盖 PowerShell 下的 `shine self`、`shine shell`、部分已适配的 app 预设，以及用 PowerShell 实现的 `shine sys init` 预设，并会同时更新 `powershell.exe` 与 `pwsh.exe` 对应的 profile。

也可以自己构建：

```bash
cargo build --release
# Binary at: target/release/shine
```

## 使用方法

### 查看可用的 shell 预设

```bash
shine shell list
```

```
Shell Preset Categories

  agent  1 script
    ccenv         Configure Claude Code to use DeepSeek in the current shell session.
                  ...

  proxy  2 scripts
    setproxy      Set HTTP/HTTPS proxy environment variables.
                  ...
    usetproxy     Unset all proxy environment variables.
                  ...

  utils  1 script
    copyfile      Copy a file's contents to the local clipboard via OSC52.
                  ...
```

### 安装 shell 预设

```bash
shine install proxy            # 自动匹配 shell/app 类别的简写
shine shell install            # 安装全部类别
shine shell install proxy      # 仅安装 proxy 类别
shine reinstall proxy          # 自动匹配类别的重装简写
shine shell reinstall proxy    # 覆盖 proxy 的受管文件和链接
```

它会把内置 shell 脚本解包到 `~/.shine/presets/shell/`，在 `~/.shine/bin/` 中创建符号链接或 Windows shim，并把 PATH 条目追加到你的 shell 配置文件（`~/.zshrc`、`~/.bashrc`、PowerShell profile 等）：

```
Shell Presets  4 created
Bin Links      4 created
```

安装全部 shell 预设时会包含 `agent`，该类别在使用前需要在当前 env 配置中提供 `DEEPSEEK_API_KEY` 或 `DEEPSEEK_API_KEY_GPG_SECRET`。
重复运行 `install` 是安全的：已存在的文件、正确的符号链接以及已配置好的 PATH 条目都会被跳过。若你想覆盖受管预设文件、链接和 shell 配置中的 PATH 条目，请使用 `reinstall`。

顶层的 `install`、`reinstall` 和 `uninstall` 命令需要一个类别名，并会自动路由到 `shell/<category>` 或 `app/<category>`。如果 shell 和 app 预设中存在同名类别，`shine` 会提示你选择其中一个。

shell 元数据可以通过 `platforms = ["unix"]` 或 `platforms = ["windows"]` 只在特定平台暴露某些条目。内置的 `agent` 类别就使用了这个机制：Unix shell 下的 `ccenv` 来自 `cc.sh`，Windows PowerShell 下的 `ccenv` 来自 `cc.ps1`。

在 Windows 上，PowerShell 的 PATH 注入会同时更新这两个 profile 文件，确保 `powershell.exe` 和 `pwsh.exe` 都能看到同一条 `~/.shine/bin` 配置：

- `~/Documents/PowerShell/Microsoft.PowerShell_profile.ps1`
- `~/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1`

### 卸载 shell 预设

```bash
shine uninstall proxy              # 自动匹配 shell/app 类别的简写
shine shell uninstall                # 卸载全部类别
shine shell uninstall proxy          # 仅卸载 proxy 类别
shine shell uninstall --dry-run      # 预览，不执行变更
shine shell uninstall --purge        # 同时删除空的受管目录
shine shell uninstall proxy --purge  # 卸载 proxy 并删除其预设目录
```

卸载会移除 `~/.shine/bin/` 中由 `shine` 管理的符号链接或 shim、`~/.shine/presets/shell/` 中的预设文件，以及 shell 配置中的 PATH 条目。用户自行创建的文件不会被删除。

如果指定了类别，只会移除该类别的文件和链接；PATH 条目会被保留，以便其它已安装类别继续可用。

`--purge` 会删除目标目录：未指定类别时删除整个 `~/.shine/presets/shell/` 树，指定类别时只删除 `~/.shine/presets/shell/<category>/`。它不会删除 `~/.shine/config.toml`，也不会删除根目录 `~/.shine/`。

### Shell 补全

```bash
shine completions install
```

打开新 shell，或手动重新加载一次 shell 配置（`source ~/.zshrc` 或 `source ~/.bashrc`）。

安装或重装某个具体 shell 预设（例如 `shine shell install proxy`）时，也会在刷新 managed shell profile 的同时更新补全配置。

如果需要手动配置或检查脚本，`shine completions <shell>` 仍会把 `bash`、`zsh` 和 `powershell` 的注册脚本输出到 `stdout`。

### 查看可用的应用预设

```bash
shine app list
```

```
App Preset Categories

  JetBrains  JetBrains IDEs configuration.
  ghostty    Ghostty terminal configuration.
  git        Personal git configuration with common aliases and sensible defaults.
  starship   Starship prompt: minimal left-prompt with git branch and status.
  vim        Vim configuration directory with base config and machine-local overrides.  2 files

Run `shine app install <CATEGORY>` to install a specific category.
Run `shine app install` to install all.
```

### 查看可用的系统初始化预设

```bash
shine sys list
```

它会列出内置的操作系统初始化预设，并用 `▶` 标记当前平台。

### 运行当前操作系统的初始化流程

```bash
shine sys init
shine sys init --preset recommended
shine sys init --dry-run
shine sys status
```

`shine sys init` 会检测当前操作系统，读取 `presets/sys/<os>/shine.toml`，解析出待执行的安装项，然后对每个选中的 item 分别调用一次当前平台的初始化脚本。所有 item 成功完成后，`shine` 会在 Rust 侧刷新受管 shell profile 集成。

- 在 TTY 中，`shine sys init` 会打开一个交互式多选界面，默认值来自预设的 `default_profile`
- `shine sys init --preset <PROFILE>` 会跳过交互，直接应用指定 profile
- 非 TTY 环境下，`shine sys init` 会回退到 `default_profile`
- `shine sys init --dry-run` 会输出解析后的项目、逐项脚本调用命令、内部 profile 更新步骤，以及脚本内容，但不会执行
- `shine sys status` 会显示当前操作系统此前记录过的初始化项目

系统初始化预设使用如下元数据结构：

```toml
description = "Initialize Ubuntu system with selectable setup steps."
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install the latest stable Neovim release."

[profiles.recommended]
items = ["neovim"]
```

初始化脚本可以输出一行机器可读状态，让 `shine` 渲染紧凑摘要：

```bash
printf 'SHINE_SYS_STATUS\t%s\t%s\n' "already-installed" "nvim found"
```

支持的状态包括 `installed`、`already-installed`、`skipped`、`updated`、`needs-action`、`completed` 和 `failed`。其他脚本输出会作为当前 item 的缩进日志保留。没有输出状态行的旧脚本仍可运行；成功时会显示为 `completed`。

当前内置预设：

- `ubuntu` — 提供 Neovim、AstroNvim、Atuin、Yazi、Starship、zoxide、zsh-vi-mode、fzf、bat、eza、pnpm、mise、Homebrew 和 ZeroTier 的可选步骤。`recommended` profile 包含核心编辑器、历史记录、文件管理器、提示符、目录跳转和 shell 工具步骤；pnpm、mise、Homebrew 和 ZeroTier 通过 `all` profile 或显式选择启用。
- `macos` — 提供 Homebrew、Rust、Yazi、Starship、Neovim、AstroNvim、ZeroTier、zsh 插件、zoxide、Atuin、fzf、bat、eza、nvm、Bun、pnpm、mise 和 Fastfetch 的可选步骤。`recommended` profile 包含 Homebrew 和核心终端/编辑器工具；`all` profile 额外包含 JavaScript 运行时、mise 和 Fastfetch。
- `windows` — 提供 Rust、Yazi、Starship、zoxide、Atuin、fzf、bat、eza、ZeroTier、Bun、pnpm 和 mise 的可选步骤。`recommended` profile 包含 Rust 和核心终端工具；`all` profile 额外包含 JavaScript 运行时和环境管理器步骤。

当所选工具需要 shell 集成时，sys init 会安装受管的 `pre` 和 `post` profile loader。`pre` loader 会放在用户 profile 靠前位置，用于 PATH、Homebrew 和补全搜索路径；`post` loader 会放在靠后位置，用于 Yazi、Starship、zoxide、Atuin、fzf、mise、别名和 shell 插件。受管 profile 文件会被合并，用户在其中的修改会保留或提示需要检查。

在 Ubuntu 和 macOS 上，受管的 `pre` profile 还会通过 OSC 11 查询交互式终端的背景色，并导出 `SHINE_TERMINAL_THEME=light|dark`。它会同步设置 bat：浅色背景使用 `GitHub`，深色背景使用 `OneHalfDark`。在受管 profile 加载前设置 `SHINE_SYNC_TERMINAL_THEME=0` 可关闭此功能；使用 `SHINE_BAT_LIGHT_THEME` 和 `SHINE_BAT_DARK_THEME` 可自定义对应主题。查询失败或终端不支持时会静默跳过；macOS 的 sys profile 仍仅管理 zsh，Ubuntu 支持 bash 和 zsh。

### 查看应用预设详情

```bash
shine app info starship
shine app info ghostty
shine app info vim
```

会输出单个类别的描述、目标位置和文件列表；如果该类别已经安装，还会给出每个文件的安装状态。

### 安装应用预设

```bash
shine install starship        # 自动匹配 shell/app 类别的简写
shine app install             # 安装全部应用类别
shine app install ghostty     # 仅安装一个类别
shine app install starship    # 仅安装一个类别
shine app install --dry-run   # 预览目标写入
shine reinstall ghostty       # 自动匹配类别的重装简写
shine app reinstall ghostty   # 覆盖一个类别的受管文件
```

`shine app install` 会先把内置文件解包到 `~/.shine/presets/app/`，然后复制到最终目标位置。

```
Installing  4 files available
  ✓  config.ghostty  →  ~/.config/ghostty/config.ghostty
  ✓  gitconfig   →  ~/.gitconfig
  ✓  starship.toml  →  ~/.config/starship/starship.toml
  -  vimrc  already up to date

Done  3 installed · 1 skipped
```

如果存在 `presets/app/<CATEGORY>/shine.toml`，该类别会使用目录级元数据：

```toml
description = "Vim configuration directory"
dest = "~/.vim"
```

当 `shine.toml` 定义了 `files` 时，只安装列出的条目。若省略 `files`，`shine` 会把整个类别目录视为受管内容，并把除 `shine.toml` 之外的所有文件按相同相对路径映射到 `dest`。

`shine app install` 写入文件前，`dest` 必须展开为当前平台的绝对路径。元数据也可以使用平台映射，让同一类别在 Unix 和 Windows 上解析到不同目标根目录：

```toml
[dest]
windows = "~/.docker"
unix = "/etc/docker"
```

#### 文件变换

`[[files]]` 条目可以声明一个 `transforms` 管道，在写入目标前处理源文件。如果变换会改变输出格式，可用 `target` 修改目标文件名：

```toml
description = "Docker Engine daemon configuration"

[dest]
windows = "~/.docker"
unix = "/etc/docker"

[[files]]
source      = "daemon.jsonc"
target      = "daemon.json"
description = "Docker Engine daemon options"
transforms  = ["jsonc-to-json"]
```

`shine app install` 的输出会显示变换步骤：

```
  ✓  daemon.jsonc  [jsonc-to-json]  →  ~/.docker/daemon.json
```

`shine update` 比较的是**变换后的最终输出**和已安装文件，因此即使源文件变了，只要生成出的 JSON 完全一致，也会被报告为 **up-to-date**。

对于需要保留其它用户设置的 JSON 配置文件，`[[files]]` 条目也可以改用“受管键合并”，而不是整文件覆盖：

```toml
[[files]]
source = "settings-store.jsonc"
target = "settings-store.json"
transforms = ["template", "jsonc-to-json"]
install_mode = "json-merge"
managed_keys = ["proxy", "containersProxy"]
```

`json-merge` 会把变换后的源文件当作 JSON 对象，只更新目标文件里列出的顶层键；卸载时也只删除这些同名键。

内置的 `docker-engine` app 预设只管理 Docker Engine daemon 配置。它在 Windows 上写入 `~/.docker/daemon.json`，在 Unix 上写入 `/etc/docker/daemon.json`。这个路径表示的是 Docker Engine daemon 配置，不等同于 Docker Desktop 的代理设置文件。

内置的 `docker-desktop` app 预设第一版只在 Windows 上提供。它会把 Docker Desktop 代理设置合并进 `~/AppData/Roaming/Docker/settings-store.json`，并且只管理 `proxy` 与 `containersProxy` 两个键，其它 Docker Desktop 设置保持不变。

通过 `# shine-template: true` 启用模板替换的 shell 脚本也按同样方式检查。`shine update` 会使用当前 `[env]` 值重新渲染源脚本；如果渲染结果与已安装脚本不同，就会报告 `update available`，包括源脚本来自外部 `presets_dir` 的情况。

**支持的变换**

| Name | From | To | Description |
|---|---|---|---|
| `jsonc-to-json` | `.jsonc` | `.json` | 去除 `//` 和 `/* */` 注释、尾随逗号，并输出规范 JSON |

单步和多步变换都通过同一个 `transforms` 数组声明：

```toml
transforms = ["jsonc-to-json"]
```

为了兼容旧配置，也接受 `transform = "jsonc-to-json"` 这种单步写法，但新预设应优先使用 `transforms = [...]`。

如果不存在 `shine.toml`，`shine` 会回退到旧的文件级规则：预设文件可以通过开头的 `shine-dest:` 注解指定一个在 `~` 展开后的显式绝对目标路径。若没有该注解，安装目标默认是：

```text
<app_default_dest_root>/<CATEGORY>/<FILE>
```

默认 `app_default_dest_root` 为 `~/.config`。

如果目标文件已经存在且不受 `shine` 管理，安装前会先把它移到 `*.shine.bak`。已安装的应用文件会记录到 `~/.shine/app-manifest.toml` 中，因此重复安装时可以安全跳过未变化文件，只覆盖那些此前由 `shine` 安装过的文件。

### 卸载应用预设

```bash
shine app uninstall                # 卸载全部应用类别
shine app uninstall ghostty        # 仅卸载 ghostty 类别
shine app uninstall starship       # 仅卸载 starship 类别
shine app uninstall --dry-run      # 预览，不执行变更
shine app uninstall --purge        # 同时删除预设和 manifest
shine app uninstall git --purge    # 卸载 git 并删除其预设目录
```

卸载时只会删除那些内容仍与 `~/.shine/app-manifest.toml` 中记录版本一致的应用文件。如果文件在安装后被修改过，`shine` 会保留该文件，并标记为用户修改。若安装时曾为非受管文件创建备份，卸载时会自动恢复。

如果指定了类别，只会移除该类别的受管文件；其它已安装类别不受影响。

`--purge` 还会在指定类别时删除 `~/.shine/presets/app/<category>/`，未指定类别时删除整个 `~/.shine/presets/app/` 和 `~/.shine/app-manifest.toml`。

### 列出已安装的预设和配置

```bash
shine list
```

只显示当前已经安装或配置好的内容，用于快速回答“这台机器上现在启用了什么”。未安装项会被省略，也不会展示额外状态细节。
如果某个 shell 预设的源文件存在，但命令符号链接缺失，它也会被省略，因为此时它无法通过 `~/.shine/bin/` 调用。

```
Shell Presets
  proxy/setproxy
  proxy/usetproxy

App Configs
  git       →  ~/.gitconfig
  ghostty   →  ~/.config/ghostty
  starship  →  ~/.config/starship/starship.toml
```

如果当前没有安装任何内容，`shine list` 会提示运行 `shine shell install` 或 `shine app install`。

### 检查已安装配置详情

```bash
shine info git
shine info starship
shine info proxy
shine info setproxy
shine info git --verbose
```

会显示受管应用配置或 shell 预设的元数据、彩色状态，以及在适用时显示预期内容差异。加上 `--verbose` 后，还会输出已安装或渲染后的文件内容。目标名称会与已安装类别、命令名、显示名、源文件名和目标文件 basename 进行匹配。若短名称有歧义，请使用报错中提示的规范形式：

```bash
shine info app/git
shine info shell/proxy/setproxy
```

对于应用配置，`shine info --verbose` 读取的是已安装目标文件。对于 shell 预设，它读取的是实际生效的脚本目标；如果脚本使用了模板渲染，则会读取 `~/.shine/rendered/` 下对应的渲染结果。

### 更新状态和版本检查

```bash
shine update
shine update --verbose
```

只显示已安装配置里存在可用更新的条目，然后再检查是否有更新的 `shine` 发行版。加 `--verbose` 后，会把已是最新或需要关注的安装项一并列出：

```
Shell Presets
  ↑  proxy/setproxy       update available  run `shine upgrade`

App Configs
  ↑  starship           →  ~/.config/starship/...     update available  run `shine upgrade`
```

状态符号：

| Symbol | Meaning |
|--------|---------|
| `✓` | 已安装且为最新 |
| `↑` | 有可用更新，运行 `shine upgrade` |
| `~` | 用户修改过或部分安装 |
| `!` | 目标文件缺失（曾安装过） |
| `✗` | 未安装 |

### 导出并自定义预设

```bash
shine export
```

会把所有内置 shell 脚本和应用配置复制到当前配置的 `presets_dir`（默认是 `~/.shine/presets/`）。导出后你可以自由修改这些文件；后续安装时，`shine` 会优先读取文件系统中的副本，而不是二进制内置资源。

如果想通过 CLI 把 `shine` 切换到自定义预设目录：

```bash
shine link ~/dotfiles/shine-presets --create
shine export
```

也可以在 `~/.shine/config.toml` 中设置 `presets_dir`：

```toml
presets_dir = "~/dotfiles/shine-presets"
```

然后把默认预设导出过去，作为初始版本：

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine export
```

当配置了 `presets_dir` 后，所有 `install`、`update` 和 `list` 命令都会自动从外部目录读取。每个命令输出中都会显示当前激活的预设来源，避免你混淆实际使用的是哪份文件。

如果只想做少量自定义，可以使用 presets overlay。Overlay 文件会按相同相对路径覆盖内置预设，例如 `app/starship/starship.toml` 或 `shell/proxy/set_proxy.sh`。完整外部 `presets_dir` 的优先级高于 overlay。

```bash
shine overlay link ~/dotfiles/shine-overlay --create
shine overlay show
shine overlay unlink
```

### 初始化一个预设目录

```bash
cd ~/dotfiles/shine-presets
shine init
```

`shine init` 会在当前目录创建 `shine.config.toml`，并写入 `presets_dir = "."`，这样该文件可以提交到 Git，并在另一台机器上复用。之后只要从该目录或其子目录运行命令，`shine` 就会向上查找最近的 `shine.config.toml`，并以该文件所在目录为基准解析相对路径；同时，诸如 `bin/`、模板渲染脚本、更新检查缓存和应用 manifest 等运行时状态，仍保存在 `~/.shine/` 下。

该命令在写入前会请求确认。脚本场景可使用 `shine init --yes`。

### 运行时更新策略

`shine` 在执行命令前会检查 `biulight/shine` 的最新 GitHub Release，并把结果缓存 24 小时到 `~/.shine/`。

- 发现更高的 `major` 或 `minor` 版本：打印升级提醒，但继续执行
- 发现更高的 `patch` 版本：要求先运行 `shine self upgrade`，之后才能继续
- 网络或 API 失败：静默跳过，命令继续执行
- 缓存写入尽力而为：如果 `~/.shine/update-check.json` 无法写入，本次命令仍会使用刚查到的结果

手动命令：

```bash
shine update        # 显示可用配置更新，然后强制检查最新 release
shine update --verbose  # 同时显示已是最新和非更新类状态
shine self upgrade  # 下载并安装当前平台的最新稳定版
shine self upgrade --channel stable   # 显式重装稳定版
shine self upgrade --channel preview  # 安装持续滚动的 preview 预发布版
shine upgrade       # 强制更新已安装的 shell 和应用配置
shine upgrade --verbose  # 包含 env 模板检查细节
```

preview 升级来自固定的 `preview` GitHub 预发布版本，自动更新检查不会使用这个通道。如果当前已安装的 preview 与当前预发布构建一致，`shine self upgrade --channel preview` 会报告已是最新，而不会重复安装。preview 二进制会在 `shine --version` 中用 SemVer build metadata 标识，例如 `0.35.0+preview.abc1234`；稳定版则继续显示 `0.35.0`。

如果 `~/.shine/` 下的缓存目录不存在，`shine` 会在保存更新检查缓存前自动重建它。

### 安装脚本选项

`install.sh` 默认把 `shine` 安装到 `~/.local/bin/shine`，不会修改你的 shell 配置。

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=0.35.0 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

`install.ps1` 默认把 `shine.exe` 安装到 `%LOCALAPPDATA%\Programs\shine\shine.exe`，不会修改你的用户 PATH。

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "0.35.0"; .\install.ps1
$env:SHINE_REPO = "biulight/shine"; .\install.ps1
```

## 内置预设

### app/ghostty

内置的 Ghostty 预设会安装主配置 `config.ghostty`，以及位于 `~/.config/ghostty/themes/` 下成对的亮色和暗色主题。默认配置使用自动明暗切换：

```text
theme = light:Shine Light,dark:dark_Alien Blood
```

如果你希望内置亮色和暗色主题在安装或 `shine upgrade` 时渲染出背景图片路径，可通过 `shine env set` 设置 `GHOSTTY_BG_LIGHT` 和 `GHOSTTY_BG_DARK`。

### shell/proxy — `setproxy` / `usetproxy`

用一组命令管理当前终端会话的代理配置。

**设置代理：**

```bash
setproxy           # 自动检测 SOCKS5，不行则回退到 HTTP
setproxy sock5     # 强制使用 SOCKS5
setproxy http      # 强制使用 HTTP
```

如果刚执行过 `shine shell install proxy`，请先重新加载一次 shell 配置（例如 `source ~/.zshrc` 或 `. $PROFILE`），或打开一个新的 shell，然后再直接使用 `setproxy`。

会同时配置：
- shell 环境变量（`http_proxy`、`https_proxy`、`all_proxy` 等）
- npm 兼容的进程配置（`npm_config_proxy`、`npm_config_https_proxy`），供 npm 和 pnpm 使用
- Git 兼容的代理环境变量

Yarn 是例外：如果检测到 Yarn，`setproxy` 会打印提示并更新 Yarn 代理配置，因为 Yarn 代理设置不能可靠地限制在当前 shell 会话中。

默认端口：HTTP `6152`，SOCKS5 `6153`（如需修改，请编辑 `~/.shine/config.toml` 中的 `[env]`）。

**取消代理：**

```bash
usetproxy
```

会清除当前会话中的代理环境变量。如果已安装 Yarn，也会删除 `setproxy` 可能写入的 Yarn 代理配置项。

### shell/utils — `copyfile` / `shine-env-export`

面向终端工作流的小工具命令。内置的 Unix `copyfile <file>` 命令会通过 OSC52 把文件内容复制到本地剪贴板，适合在 SSH 或支持 OSC52 剪贴板集成的终端复用器中使用。

先运行 `shine shell install utils` 安装跨 shell 的 env helper，之后无需手写 `eval` 或 `Invoke-Expression`，即可把 Shine env 值载入当前 shell：

```bash
shine-env-export MY_TOKEN
shine-env-export MY_TOKEN --as API_TOKEN
```

该 helper 会优先读取并解密 `MY_TOKEN_SECRET`，不存在时回退到明文 `MY_TOKEN`。`--as API_TOKEN` 只会改变导出到 shell 的变量名。

### shell/agent — `ccenv`

为 Claude Code + DeepSeek provider 配置当前 shell 环境。

把你的 key 写入全局 env 覆盖文件 `~/.shine/shine.env.toml`，或者项目本地 `shine.config.toml` 同目录下的 env 文件：

```toml
DEEPSEEK_API_KEY = "..."
```

也可以改为存储 base64 编码后的 GPG 密文：

```toml
DEEPSEEK_API_KEY_GPG_SECRET = "<base64-gpg-ciphertext>"
```

用你现有的 GPG key 生成该加密值。如果私钥托管在 YubiKey 上，`gpg-agent` 会在执行 `ccenv` 时处理 PIN / touch 提示：

```bash
shine env encrypt --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_GPG_SECRET
```

`shine env encrypt` 默认使用 `config.toml` 中的 `gpg_key_id`。如需单次覆盖，可传入 `-r/--recipient <key-id>`。

也可以直接解密当前 env 配置中的任意 base64 GPG secret：

```bash
shine env decrypt DEEPSEEK_API_KEY_GPG_SECRET
```

如果要把值导出到当前 shell，可直接生成 shell 代码，也可以通过 `--as` 改用另一个变量名：

```bash
eval "$(shine env export MY_TOKEN)"
eval "$(shine env export MY_TOKEN --as API_TOKEN)"
```

安装 `utils` 预设后，也可以用 `shine-env-export MY_TOKEN --as API_TOKEN` 直接应用，无需手写 `eval`。

然后安装并使用这个 helper：

```bash
shine shell install agent
ccenv
```

如果同时设置了 `DEEPSEEK_API_KEY_GPG_SECRET` 和 `DEEPSEEK_API_KEY`，会优先使用加密 secret。若 GPG 解码或解密失败，`ccenv` 会直接停止，而不会回退到明文 key。

### Shell 预设元数据

Shell 预设类别可以可选定义 `presets/shell/<category>/shine.toml`，用于控制安装后的命令名：

```toml
description = "Proxy helper commands"

[[files]]
source = "set_proxy.sh"
target = "setproxy"
needs_source = true
platforms = ["unix"]

[[files]]
source = "set_proxy.ps1"
target = "setproxy"
needs_source = true
platforms = ["windows"]
```

`source` 指向类别目录下实际存储的脚本文件。`target` 控制链接到 `~/.shine/bin/` 的命令名。若省略 `target`，`shine` 会回退到脚本文件名 stem。`platforms` 可选，支持 `unix` 和 `windows`；省略时表示所有平台。

## 配置

`~/.shine/config.toml` 会在首次运行时自动创建。全局配置继续使用通用文件名 `config.toml`，因为 `~/.shine/` 本身已经是专属目录。项目本地预设仓库则可以额外使用 `shine.config.toml`，避免与其他工具的 `config.toml` 冲突。

运行时覆盖目录：

```bash
SHINE_CONFIG_DIR=/custom/path shine shell install   # 同时覆盖 shine 目录和 presets 目录
SHINE_PRESETS=/custom/presets shine shell install   # 仅覆盖 presets 目录
```

也可以把自定义预设目录持久化写入 `~/.shine/config.toml`：

```toml
presets_dir = "/custom/presets"
```

配置发现逻辑会从当前目录开始向父目录查找 `shine.config.toml`。如果找不到，仍会兼容识别包含 `presets_dir` 的旧式项目 `config.toml`，但会给出警告。再找不到时，`shine` 才使用 `~/.shine/` 或 `SHINE_CONFIG_DIR` 下的全局配置。

预设来源优先级为：`SHINE_PRESETS` > 当前激活配置里的 `presets_dir` > 默认目录。当设置了 `SHINE_CONFIG_DIR` 且没有激活项目配置时，默认预设目录会变成 `$SHINE_CONFIG_DIR/presets`。

对于没有 `shine-dest:` 注解的应用预设，你也可以修改默认安装根目录：

```toml
app_default_dest_root = "~/.config"
```

为 `shine env encrypt` 设置默认 GPG recipient：

```toml
gpg_key_id = "<key-id>"
```

模板变量放在 `[env]` 表里：

```toml
[env]
HTTP_PROXY_PORT = "6152"
SOCKS5_PROXY_PORT = "6153"
PROXY_HOST = "127.0.0.1"
PROXY_NO_PROXY = "localhost,127.0.0.1,::1"
GHOSTTY_BG_LIGHT = ""
GHOSTTY_BG_DARK = ""
```

`shine env show` 会显示当前 preset catalog 提供的变量说明，并默认隐藏敏感值；需要查看完整值时可使用 `--reveal`。如果需要在当前配置中覆盖说明，可以把值和说明写在同一个 inline table 中：

```toml
[env]
MY_API_TOKEN = { value = "secret", description = "内部 API 的访问令牌" }
```

Preset 作者可以在 `<presets>/env.toml` 中提供共享元数据：

```toml
[[variables]]
key = "MY_API_TOKEN"
description = "内部 API 的访问令牌"
sensitive = true
```

inline description 的优先级高于 preset catalog。Catalog 只保存元数据，不保存或提供变量值。

设置 `GHOSTTY_BG_LIGHT` 和 `GHOSTTY_BG_DARK` 后，Ghostty 预设在不同外观模式下会安装带背景图片路径的主题。保留为空则表示安装内置 Ghostty 预设但不启用背景图。

全局覆盖可通过放置在 `~/.shine/shine.env.toml` 的扁平 `shine.env.toml` 文件提供。项目本地覆盖则放在 `shine.config.toml` 同目录下。`shine.env.toml` 中的值会覆盖当前配置 `[env]` 表中的同名 key，而不会改写任一文件。当全局和项目本地 env 文件同时存在时，项目本地优先。若项目本地 `shine.env.toml` 不存在，仍会兼容读取旧的 `.env.toml`。

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = "127.0.0.1"
```

## 目录布局

```
~/.shine/
├── app-manifest.toml
├── config.toml
├── shine.env.toml    # 可选的扁平 env 覆盖文件
├── bin/
│   ├── setproxy         # symlink/shim → 平台对应 proxy 脚本
│   ├── usetproxy        # symlink/shim → 平台对应 proxy 脚本
│   └── copyfile         # symlink → presets/shell/utils/copyfile.sh
└── presets/
    ├── app/
    │   ├── JetBrains/
    │   │   └── .ideavimrc
    │   ├── ghostty/
    │   │   ├── config.ghostty
    │   │   ├── themes/
    │   │   │   ├── Alien Blood
    │   │   │   ├── Github Light Default
    │   │   │   └── Shine Light
    │   │   └── shine.toml
    │   ├── git/
    │   │   └── gitconfig
    │   └── starship/
    │       └── starship.toml
    └── shell/
        ├── proxy/
        │   ├── shine.toml
        │   ├── set_proxy.ps1
        │   ├── set_proxy.sh
        │   ├── uset_proxy.ps1
        │   └── uset_proxy.sh
        └── utils/
            ├── shine.toml
            └── copyfile.sh
```

实际安装后的应用文件位于它们各自的目标路径，例如：

```text
~/.gitconfig
~/.ideavimrc
~/.config/ghostty/config.ghostty
~/.config/starship/starship.toml
```

## 开发

```bash
cargo nextest run --all-features   # 测试（pre-commit 使用它）
cargo test                         # 备用
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo fmt
cargo deny check bans licenses sources
typos
```

### 工作区布局

```
shine/
├── cli/        # binary crate — CLI 解析、命令分发、配置
│   ├── build.rs               # 监听 presets/ 变化并触发 rust-embed 重新编译
│   └── src/
│       ├── main.rs
│       ├── bin_links.rs       # 符号链接管理
│       ├── colors.rs          # 感知 TTY 的颜色辅助（在 NO_COLOR 下优雅降级）
│       ├── presets.rs         # 内置资产解包、list_categories
│       ├── apps/              # 应用预设安装/卸载、manifest、目标路径解析
│       ├── config/            # Config 结构、加载/保存、环境变量优先级链
│       ├── commands/          # clap 子命令定义
│       └── shells/            # ShellType、安装/卸载/列表、PATH 注入
├── utils/      # library crate — 保留注释的 TOML 迁移能力
└── presets/    # 编译时嵌入二进制的 shell/app 预设文件
    ├── app/
    └── shell/
```

## License

MIT OR Apache-2.0
