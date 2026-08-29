---
title: 初始化与管理系统
sidebar_position: 3
---

# 初始化与管理系统

系统预设为 macOS、Ubuntu 和 Windows 提供可选择的开发环境初始化步骤，以及少量可逆的受管系统
资源；它不是通用的机器配置或包版本管理器。实际可用项目以当前版本的 `shine sys list` 为准。

各平台 profile 的项目清单，以及 `split-dns` 所需的环境变量和安全预览步骤见[内置预设](../reference/built-in-presets.md#系统预设)。

## 先查看，再执行

```bash
shine sys list
shine sys list --all
shine sys info split-dns
shine sys bootstrap --dry-run
```

`--dry-run` 会显示选择结果、provider/脚本调用，以及每个将持久加载的 item-owned shell 集成，但不执行变更。某些初始化项目需要管理员权限或额外环境变量，`shine sys info <ITEM>` 会列出要求。

缺失的 item 即将执行时，Shine 会先显示其 `sys/<ITEM>` 标识和名称，因此后续出现的授权或密码提示会明确对应当前正在安装的软件。

## 交互选择或应用 Profile

```bash
shine sys bootstrap
shine sys bootstrap mise
shine sys bootstrap rust mise
shine sys bootstrap --item rust --item mise --yes
shine sys bootstrap --preset recommended
shine sys bootstrap --preset minimal
shine sys bootstrap --proxy --dry-run
```

- 在交互式终端中，`shine sys bootstrap` 会打开多选界面。
- 位置参数 item ID 只引导这些项目，保留输入顺序，并忽略重复项。
- 重复使用 `--item` 是供脚本与 setup orchestrator 使用的等价显式写法。
- 指定 `--preset` 时直接应用命名 profile。
- 位置参数、重复的 `--item` 与 `--preset` 不能组合；受管资源使用 `sys apply`，而不是 `sys bootstrap`。
- 非交互环境没有指定 profile 时使用预设的默认 profile。

完成选择后，实际变更会先展示绑定输入快照的安全 Plan，其中包含语义步骤、权限和输入指纹。交互确认默认为否；自动化必须传入 `--yes`。该参数只跳过提示，Plan 仍会展示并用最新输入重新验证。`--dry-run` 是更早阶段的 provider/脚本预览，不能与 `--yes` 同时使用。

Ubuntu 还提供 `minimal` profile，适合生产服务器：仅安装 Neovim、fzf、bat、eza 和 zoxide，不包含 shell 历史同步、提示符、Node.js 工具链或 Homebrew。运行前仍应先执行 `shine sys bootstrap --preset minimal --dry-run` 复核当前版本的实际步骤。

下载需要经过 HTTP 代理时，添加 `--proxy`。Shine 会根据 `[env]` 中的 `PROXY_HOST`、`HTTP_PROXY_PORT` 和 `PROXY_NO_PROXY` 为初始化脚本设置大小写两套代理变量；默认地址为 `http://127.0.0.1:6152`。先配合 `--dry-run` 检查实际注入值。

Windows 的 `winget` 不读取这些环境变量，因此 Shine 还会显式传递 `winget install --proxy`。若系统尚未启用该选项，请在管理员 PowerShell 中先运行：

```powershell
winget settings --enable ProxyCommandLineOptions
```

完成后检查记录：

```bash
shine sys status
```

这里展示的是执行记录：`installed`、`already installed` 或 `completed` 描述上一次 bootstrap
调用观察到的结果，并不会实时探测第三方软件当前是否仍存在或已是最新版。

## 用软件自己的工具升级

`shine sys bootstrap` 只负责确保软件存在，并不是软件更新管理器。Shine 不会检查或升级引导软件；
请使用拥有该软件的包管理器或上游工具，例如 Homebrew、apt、winget、mise 或 rustup。重新运行
`shine sys bootstrap` 只会确认所选软件存在，不会升级它。

内置 mise 步骤遵循同一边界：它可以安装 mise，并把激活内容加入 Shine 管理的 Shell profile，
但不会创建或更新 mise 配置，也不会管理 runtime 版本。`mise.toml`、工具安装与版本升级仍由 mise
负责。

## 管理 shell 集成状态

成功的 bootstrap 只启用本次选中 item 声明的 shell 集成，不会禁用之前运行已启用的集成。命名
selection profile 也不是软件或 shell 配置的 desired-state 替换集合。

```bash
shine sys profile disable mise --dry-run
shine sys profile disable mise
shine sys profile enable mise
```

这些命令只修改 Shine 自己生成的 profile 内容。disable 不卸载软件；enable 会先执行 item 声明的
检测，缺失时提示先 bootstrap。执行变更前会显示并重新校验绑定快照的安全 Plan；自动化调用
必须添加 `--yes`，`--dry-run` 仍是独立预览。`shine upgrade` 不再隐式修改或重新组合 profile
启用状态；需要变更该状态时请使用这些显式 profile 命令。

`shine update` 和 `shine upgrade` 仍只处理 Shine 管理的配置和受管系统资源，不会升级这些
第三方软件。

顶层 `shine list` 会列出当前操作系统已登记在 sys manifest 中的受管系统配置；它用于快速总览，详细状态仍以 `shine sys status` 和 `shine sys info <ITEM>` 为准。`update --verbose` 与 `upgrade --verbose` 会同时展示跳过、已是最新以及需要注意的受管资源。

## 受管系统项目

部分系统配置是可重复应用和安全移除的受管项目：

```bash
shine sys apply --dry-run
shine sys apply split-dns
shine sys apply split-dns --yes # 非交互批准
shine sys uninstall split-dns --dry-run
shine sys uninstall split-dns
```

非 dry-run 的 managed 操作会显示绑定快照的 Plan，确认默认是 No。`--yes` 只跳过提示，不能
跳过 Plan 展示、权限 blocker 或执行前复核。若项目需要管理员权限，会在 Plan 批准后另行请求。

需要把异地局域网中的私有域名定向到 ZeroTier DNS 时，可参考
[使用 ZeroTier、CoreDNS 和 Shine 搭建异地私有域名网络](https://blog.biulight.top/timeline/knowledge/zerotier-coredns-split-dns)。

在 Ubuntu 上，`split-dns` 依赖应用查询 `systemd-resolved` 的 `127.0.0.53` stub。Shine 会在检测到 stub 被关闭时给出警告或拒绝写入无效配置；先重新启用 `DNSStubListener`，或确认本机解析链路确实会经过 `systemd-resolved`，再应用该项目。

系统 profile 会尽量合并并保留用户内容。只有明确希望备份并替换冲突 profile 时，才使用 `shine sys bootstrap --force-profile`。

## 平台说明

- macOS 的受管 shell profile 以 zsh 为目标。
- Ubuntu 支持 bash 和 zsh。
- Windows 系统预设和 profile 集成使用 PowerShell。
- Ubuntu 与 macOS 可自动识别终端明暗背景并设置 `SHINE_TERMINAL_THEME`；设置 `SHINE_SYNC_TERMINAL_THEME=0` 可关闭。
