# shine

一个用于管理 shell 预设、应用配置和系统初始化预设的 Rust CLI。

`shine` 将可复用的 shell 脚本、应用配置预设和操作系统初始化预设打包进一个二进制中。它会把受管资产安装到 `~/.shine/`，把 shell 命令链接到 `~/.shine/bin/`，也可以把应用配置文件复制到最终目标位置。

English README: [`../README.md`](../README.md)

## 功能特性

- **内置预设** — shell 脚本和应用配置会编译进二进制；安装后不需要联网
- **外部预设目录和 overlay** — 可用 `presets_dir` 指定基础预设来源，再链接一个小型 overlay 覆盖少量预设文件
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
shine sys list --all
shine sys info split-dns
```

`shine sys list` 会列出当前操作系统可用的全部初始化项和托管项，包括已记录状态及启用命令。使用 `--all` 可查看所有受支持的操作系统。

`shine sys info <ITEM>` 会显示 item 的类型、驱动、管理员权限要求、必需环境变量名称、当前状态和下一步命令。例如，`shine sys info split-dns` 能直接说明如何启用私有 split DNS，且不会暴露环境变量的配置值。

### 运行当前操作系统的初始化流程

```bash
shine sys init
shine sys init --preset recommended
shine sys init --dry-run
shine sys status
shine sys update
shine sys update neovim --verbose
```

`shine sys init` 会检测当前操作系统，读取 `presets/sys/<os>/shine.toml`，解析出待执行的安装项，然后对每个选中的 item 分别调用一次当前平台的初始化脚本。所有 item 成功完成后，`shine` 会在 Rust 侧刷新受管 shell profile 集成。

- 在 TTY 中，`shine sys init` 会打开一个交互式多选界面，默认值来自预设的 `default_profile`
- `shine sys init --preset <PROFILE>` 会跳过交互，直接应用指定 profile
- 非 TTY 环境下，`shine sys init` 会回退到 `default_profile`
- `shine sys init --dry-run` 会输出解析后的项目、逐项脚本调用命令、内部 profile 更新步骤，以及脚本内容，但不会执行
- `shine sys status` 会显示当前操作系统此前记录过的初始化项目
- `shine sys update [ITEM] [--verbose] [--proxy]` 是只读命令：它只检查此前由 `shine sys init` 记录的引导软件，绝不会安装或升级任何软件，也不会修改 sys manifest 或 shell profile。`--proxy` 会通过预设代理执行检查；在 Windows 上会显式传递 WinGet 的 `--proxy` 参数，因为 WinGet 会忽略标准 HTTP 代理环境变量。默认只显示由包管理器确认的可用更新及可直接复制的上游升级命令；`--verbose` 还会显示已是最新和只能手动检查的项目。对于直接安装器和用户自行维护的 Git 配置，命令会明确标记为需要手动处理，而不会猜测版本。

`shine update` 与 `shine upgrade` 仍只负责协调 Shine 管理的配置和受管系统资源，不会升级第三方引导软件。复制并运行 `shine sys update` 输出的命令始终是用户自己的明确决定。

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

在 Ubuntu 和 macOS 上，受管的 `pre` profile 还会通过 `shine theme sync` 同步终端的明暗主题，导出 `SHINE_TERMINAL_THEME=light|dark`，并设置 bat：浅色背景使用 `GitHub`，深色背景使用 `OneHalfDark`（可用 `SHINE_BAT_LIGHT_THEME`/`SHINE_BAT_DARK_THEME` 覆盖）。解析顺序依次是：已导出的 `SHINE_TERMINAL_THEME`（包含 `shine ssh` 从本地终端注入的值，见下文）、`COLORFGBG`，最后是使用总截止时间（而非逐字节超时）读取的 OSC 11 直接查询。若用户已自行设置过 `BAT_THEME`，则保持不变。可在 `config.toml` 中设置 `sync_terminal_theme = false` 或设置 `SHINE_SYNC_TERMINAL_THEME=0`（环境变量始终优先）关闭自动同步；无论该开关如何，都可随时用 `shine theme sync` 手动同步，或通过 `shine shell install utils` 安装可选的 `shine-theme-sync` 命令。`shine ssh <host>` 会在连接前直接查询本地终端，因此完全不依赖远端的 OSC 查询——详见 [docs/terminal-theme-sync-prd.md](terminal-theme-sync-prd.md)。macOS 的 sys profile 仍仅管理 zsh，Ubuntu 支持 bash 和 zsh。

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

如果只想做少量自定义，可以使用 presets overlay。Overlay 会按相同相对路径覆盖当前预设来源（内置或外部），例如 `app/starship/starship.toml` 或 `shell/proxy/set_proxy.sh`。同路径文件以 overlay 为准，overlay 独有的分类也会加入基础来源。

```bash
shine overlay link ~/dotfiles/shine-overlay --create
shine overlay show
shine overlay unlink
```

如果你的 overlay 保存在 Git 仓库中，可以让 Shine 自动维护这份检出，而不必在每台机器上手动克隆。为 overlay 指定一个 Git 地址，Shine 会以 `--depth 1`（不含历史）克隆到 `~/.shine/overlay`，并始终镜像到远端最新提交：

```bash
shine overlay link --git https://github.com/you/shine-overlay.git   # 可选：--branch main
shine overlay show      # 显示 URL、分支、托管路径与克隆状态
shine pull              # 首次运行时克隆，之后强制镜像到最新提交
```

也可以直接在 `~/.shine/config.toml` 中写入地址，再执行 `shine pull`：

```toml
presets_overlay_git = "https://github.com/you/shine-overlay.git"
# presets_overlay_git_branch = "main"   # 可选；默认使用远端默认分支
```

这特别适合「一台机器维护 overlay、其余设备只消费」的场景：每台设备只需要这个地址，
无需手动 `git clone`。由于托管检出是只读镜像，`shine pull` 始终会将其重置为与远端一致
（可安全应对 rebase、force-push），并丢弃任何本地改动。如果拉取失败（例如远端不可达），
之前的检出会保持不变并继续使用。手动 `overlay link <path>` 优先于 Git 地址，两者互斥。

如果当前 preset 来源或手动关联的 overlay 由 Git 管理，Shine 也会安全地快进拉取：

```bash
shine pull             # 同步托管 overlay，并快进 preset / overlay 仓库
shine update --pull    # 先拉取并重新加载配置，再检查状态
shine upgrade --pull   # 先拉取并重新加载配置，再应用 preset
```

拉取会拒绝有未提交改动的工作区，并使用 `git pull --ff-only`；不会自动 stash、rebase、
reset 或解决冲突。非 Git 来源会被跳过，位于同一仓库的两个来源只会拉取一次。

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
shine update --pull  # 拉取 Git 管理的 preset 后再检查状态
shine self upgrade  # 下载并安装当前平台的最新稳定版
shine self upgrade --channel stable   # 显式重装稳定版
shine self upgrade --channel preview  # 安装持续滚动的 preview 预发布版
shine upgrade       # 强制更新已安装的 shell 和应用配置
shine upgrade --pull  # 拉取 Git 管理的 preset 后再应用配置
shine upgrade --verbose  # 包含 env 模板检查细节
```

preview 升级来自固定的 `preview` GitHub 预发布版本，自动更新检查不会使用这个通道。如果当前已安装的 preview 与当前预发布构建一致，`shine self upgrade --channel preview` 会报告已是最新，而不会重复安装。preview 二进制会在 `shine --version` 中用 SemVer build metadata 标识，例如 `0.39.0+preview.abc1234`；稳定版则继续显示 `0.39.0`。

如果 `~/.shine/` 下的缓存目录不存在，`shine` 会在保存更新检查缓存前自动重建它。

### 安装脚本选项

`install.sh` 默认把 `shine` 安装到 `~/.local/bin/shine`，不会修改你的 shell 配置。

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=0.39.0 sh install.sh
SHINE_REPO=biulight/shine sh install.sh
```

`install.ps1` 默认把 `shine.exe` 安装到 `%LOCALAPPDATA%\Programs\shine\shine.exe`，不会修改你的用户 PATH。

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "0.39.0"; .\install.ps1
$env:SHINE_REPO = "biulight/shine"; .\install.ps1
```

### SSH 会话内文件传输

`shine ssh` 会打开一个正常的交互式 SSH 会话（它包装了系统自带的 `ssh`，并复用你的 `~/.ssh/config`），同时建立一条回连到发起端机器的会话专属传输通道。会话内的 `shine local download`/`upload`/`status` 就通过这条通道工作——不需要再单独调用 `scp`/`rsync`。

还可以把本机当前有效 Shine 环境中明确选择的值注入远端登录 shell 或命令。Shine 自己的选项必须写在 SSH 目标之前；`KEY=ALIAS` 可在远端重命名变量：

```bash
shine ssh --with API_URL dev
shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'
shine ssh --with-secret API_TOKEN dev
```

`--with` 只读取完全同名的明文 `[env]` 键，绝不会自动解密 `KEY_SECRET`；解密后的值必须通过显式的 `--with-secret KEY[=ALIAS]` 注入。显式值会覆盖远端进程继承到的同名值，但远端登录 shell 的启动文件仍可能再次赋值。变量仅存在于本次会话，不会写入远端配置文件。密钥会暴露给远端主机，也可能被任一端具有足够权限或同用户的进程从进程参数/环境中读取。

Windows OpenSSH 远端需要显式选择 PowerShell 包装器；它通过
编码命令安全注入会话提示、终端主题和所选变量，优先使用 PowerShell 7（`pwsh.exe`），
未安装时回退到 Windows PowerShell 5.1（`powershell.exe`），不会把 POSIX 的 `env ... sh -c`
发送给 `cmd.exe`：

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN intel.mac.local
```

交互式 Windows 会话会加载所选 PowerShell 的正常 profile，因此 Shine 管理的 PATH 和
`setproxy` 等 source-command wrapper 都可用；显式远端命令仍以 no-profile 模式执行。

此模式只支持 SSH 环境注入，不创建传输隧道；该 Windows 远端会话中不支持
`shine local download`、`upload` 或 `status`。

```bash
cd ~/work/frontend
shine ssh dev                     # 建立会话；~/work/frontend 成为该会话下面这些命令的“本机目录”

# 连接成功后，在远端执行：
shine local download result.log              # 远端 ./result.log -> 本机 ~/work/frontend/result.log
shine local download output/ '~/Downloads/build/'  # 目录也可以传输（以 tar 流式传输）
shine local upload notes.txt                  # 本机 ~/work/frontend/notes.txt -> 远端当前目录
shine local upload assets/ ./public/assets/
shine local status                            # 会话 ID、连接状态、本机目录
```

源/目标参数按其所属机器解析：`download` 的第一个参数和 `upload` 的第二个参数始终是远端路径，基于远端 shell 的当前目录解析；另一个参数始终基于会话的本机目录解析（即运行 `shine ssh` 时所在的目录，之后无论在远端执行多少次 `cd` 都不变）。如果希望*另一端*展开 `~`，请给路径加引号（例如 `'~/Downloads/'`），否则本机 shell 会在 `shine` 看到参数之前就把它展开。

两个命令都会默认把内容写入目标端工作目录下、与源同名的位置；默认拒绝覆盖已存在的目标，除非传入 `--force`；并支持 `--dry-run` 预览传输而不实际拷贝数据。连接终端时进度以单行覆写方式显示；管道/非交互场景则只输出最终一行结果。没有传输任务时，`shine local status` 也可以当作会话的存活检测使用。

`shine local` 的本机侧（运行 `shine ssh` 的那台机器）在 Windows 上同样可用；其传输协议远端仍要求 Linux 或 macOS。

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

#### age + Apple Touch ID（Secure Enclave）

如果密文需要提交到仓库中共享，并让多个团队成员各自解密——而不仅限于使用 GPG 的人——`shine env
encrypt`/`decrypt`/`seal` 还支持第二种后端 [age](https://github.com/FiloSottile/age)，并可通过
[age-plugin-se](https://github.com/remko/age-plugin-se) 在 macOS 上启用 Touch ID：

```bash
brew install age age-plugin-se   # 或使用你偏好的包管理器

# 生成一个绑定 Touch ID 的 Secure Enclave 身份，解密时会弹出系统 Touch ID 提示
shine env identity init --touch-id

# 或者生成一个在任意系统上都可用的普通身份
shine env identity init
```

`identity init` 会打印该身份对应的 `age1...`/`age1se1...` recipient。把每位团队成员的
recipient（各自的 age 或 Secure Enclave 身份）加入 `age_recipients`，这样任何一人都能解密你加密
过的内容：

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
```

```bash
shine env encrypt --backend age --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_SECRET
shine env decrypt DEEPSEEK_API_KEY_SECRET   # 如果身份是 Secure Enclave，会弹出 Touch ID 提示
```

age 后端产生的密文带有标签（`age:...`），因此 `shine` 始终能判断该用哪个后端解密——已有的、不带
标签的 GPG 密文不受影响，照常可用。`-r/--recipient` 对两种后端都可重复传入，因此一次
`encrypt`/`seal` 就能同时面向多个 recipient 加密。从 `age_recipients` 中移除某个 recipient，并
不会追溯撤销其对已提交到 git 历史中的密文的访问权限——需要重新执行 `seal` 才能轮换。

如果要把值导出到当前 shell，可直接生成 shell 代码，也可以通过 `--as` 改用另一个变量名：

```bash
eval "$(shine env export MY_TOKEN)"
eval "$(shine env export MY_TOKEN --as API_TOKEN)"
```

安装 `utils` 预设后，也可以用 `shine-env-export MY_TOKEN --as API_TOKEN` 直接应用，无需手写 `eval`。

如果只想把变量提供给一个子进程、而不修改当前终端，可使用 `env run` 中可重复的
`--with` 参数：

```bash
shine env run --with MY_TOKEN -- bun run build
shine env run --with MY_TOKEN=API_TOKEN -- bun run build
shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build
```

每个变量都沿用 `env export` 的规则，优先解密 `<KEY>_SECRET`，否则读取明文 `<KEY>`。
等号右侧可指定子进程看到的变量名。显式 `--with` 值会覆盖当前终端和 workspace 中的
同名变量；只要指定了至少一个 `--with`，就不要求存在 workspace 文件。

### Workspace 环境运行器

如果项目不希望保留明文 dotenv 文件，可以创建 `shine.workspace.toml`：

```toml
version = 1

[env]
modes = ["development", "production"]
default_mode = "development"
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
recipient = "alice@example.com"
```

每个环境源文件可以同时包含明文值和加密值：

```toml
version = 1

[plain]
VITE_APP_NAME = "My App"

[secret]
DATABASE_URL = true         # 保留已有密文值
API_TOKEN = false           # 下次 seal 时安全提示输入
SENTRY_TOKEN = "new-value" # seal 后自动替换成 true

[payload]
data = "<由 Shine 管理的 GPG 密文>"
```

封存待处理的值，再用合并后的环境运行命令：

```bash
shine env seal
shine env run --mode production -- bun run build
```

环境源按配置顺序合并，后面的文件覆盖前面的文件。默认保留当前进程中已存在的变量；设置
`env.override_process_env = true` 后改由 workspace 值覆盖。与 workspace 同时使用时，显式
`--with` 值始终优先。`env run` 会在系统缓存目录中自动维护按
mode 区分的加密缓存；workspace、源文件内容或覆盖顺序变化后，缓存会自动重建，不需要单独的
compile 命令。

个人覆盖文件应加入 `.gitignore`：

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

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

配置发现逻辑会从当前目录开始向父目录查找 `shine.config.toml`。如果找不到，仍会兼容识别包含 `presets_dir` 的旧式项目 `config.toml`，但会给出警告。该旧文件名将在 v0.40.0 停止支持，请将其改名为 `shine.config.toml`。项目配置是 `~/.shine/` 或 `SHINE_CONFIG_DIR` 下全局配置之上的稀疏覆盖层：项目未声明的字段继承全局值，明确声明的字段则以项目值为准。相对路径以定义该字段的配置文件所在目录为基准解析。保存项目设置时，不会把继承的全局值复制到项目文件中。

预设来源优先级为：`SHINE_PRESETS` > 项目 `presets_dir` > 全局 `presets_dir` > 默认目录。`SHINE_CONFIG_DIR` 用于选择全局配置和运行时状态目录，其默认预设目录为 `$SHINE_CONFIG_DIR/presets`。

对于没有 `shine-dest:` 注解的应用预设，你也可以修改默认安装根目录：

```toml
app_default_dest_root = "~/.config"
```

为 `shine env encrypt` 设置默认 GPG recipient：

```toml
gpg_key_id = "<key-id>"
```

或者把 age 设为默认后端，并配置其 recipient / 身份文件（参见上文 age + Apple Touch ID）：

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"   # 可选；也是默认路径
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

环境变量按 key 依次合并：内置默认值、全局 `[env]`、项目 `[env]`、全局 `shine.env.toml`、当前 presets overlay 的 `shine.env.toml`、项目 `shine.env.toml`。

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

全局覆盖可通过放置在 `~/.shine/shine.env.toml` 的扁平 `shine.env.toml` 文件提供。项目本地覆盖则放在 `shine.config.toml` 同目录下。`shine.env.toml` 中的值会覆盖当前配置 `[env]` 表中的同名 key，而不会改写任一文件。当全局和项目本地 env 文件同时存在时，项目本地优先。若项目本地 `shine.env.toml` 不存在，仍会兼容读取旧的 `.env.toml`；该兼容将在 v0.40.0 移除，请将文件改名为 `shine.env.toml`。

通过 `shine overlay link <path>` 关联的有效 overlay 目录也可以包含扁平的
`<path>/shine.env.toml`。其中的值会覆盖全局 env，并可在任意工作目录下生效；
项目本地 `shine.env.toml` 仍拥有更高优先级。该文件会在每次运行时重新读取，
无需项目级 `shine.config.toml`；执行 `shine overlay unlink` 后即停止生效。
Overlay 也可以与完整的外部 presets 来源同时使用：同路径文件以 overlay 为准，
其余文件继续来自外部 presets。

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "本地代理主机" }
```

与配置文件的 `[env]` 条目一样，扁平 override 中的每个值既可以是字符串，也可以是
inline `{ value, description }` 表。详细项会同时覆盖值和说明；字符串只覆盖值，并
保留从低优先级配置或 preset catalog 继承的说明。无效的值类型会明确报错，不会被
静默忽略。

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
