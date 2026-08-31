---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页适用于 Shine 1.8.0。任何子命令都可以使用 `--help` 查看当前安装版本的准确参数。

## 1.0 target 规则

日常命令使用 `app/<category>`、`shell/<category>`、`shell/<category>/<command>` 和
`sys/<item>` 作为规范 target。install 与 uninstall 支持 Shell 命令 target；upgrade 则在所属
类别内协调已经安装的命令。名称在 app 与 shell 间唯一时，安装和卸载也接受裸类别名；裸
Shell 命令名只用于查看。脚本和文档中建议始终写完整 target，避免以后新增同名类别后产生
歧义。

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

从 1.0 起，`reinstall` 已由 `install --replace-managed` 取代；旧的 `clear`、`pull`、`export`、`link`、`overlay` 顶层入口以及 `app build/unbuild`、`sys init`、`env show` 不再保留兼容别名。

## 顶层命令

| 命令 | 作用 |
| --- | --- |
| `shine init [--yes]` | 在当前项目创建 `shine.config.toml` |
| `shine shell <SUBCOMMAND>` | 管理 Shell 命令预设 |
| `shine app <SUBCOMMAND>` | 管理应用配置预设 |
| `shine install <TARGET> [--replace-managed] [--yes]` | 安装或修复一个 app/shell target |
| `shine uninstall <TARGET> [--force] [--purge] [--dry-run] [--yes]` | 卸载一个 app/shell target |
| `shine completions <SUBCOMMAND>` | 生成或安装 Shell 补全 |
| `shine list [--available [KIND]]` | 列出已安装资源，或用 `app`、`shell`、`sys` 浏览可用资源目录 |
| `shine info <TARGET> [--diff] [--verbose]` | 查看可用或已安装的 app/shell target，或 `sys/<ITEM>` |
| `shine update [TARGET]` | 检查受管内容和 Shine 稳定版更新 |
| `shine upgrade [TARGET] [--yes]` | 应用全部或指定 app、shell、受管 sys 更新 |
| `shine preset <SUBCOMMAND>` | 管理预设来源、overlay、导出和 Git 同步 |
| `shine state migrate [--dry-run]` | 迁移并清理旧版 Shine 运行时状态 |
| `shine trust <SUBCOMMAND>` | 查看、授予、列出或撤销 target-scoped 外部代码信任 |
| `shine self <SUBCOMMAND>` | 安装或升级 Shine 程序 |
| `shine serve <SUBCOMMAND>` | 通过本地 HTTP 服务发布 `~/.shine/http/` 下的资源 |
| `shine env <SUBCOMMAND>` | 管理预设变量、workspace 环境、代理与密钥 |
| `shine sys <SUBCOMMAND>` | 管理系统引导与受管系统配置 |
| `shine theme sync` | 解析终端明暗主题并输出 shell `export` 语句 |
| `shine ssh ...` / `shine local ...` | 开启 SSH 会话、按需代理密钥并在 POSIX 远端传输文件 |
| `shine task <SUBCOMMAND>` / `shine run <NAME>` | 保存和运行个人快捷命令 |

所有命令都支持全局 `--config-dir <PATH>`，用于临时选择全局配置和运行时状态目录。

## Shell 与 App

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes]
shine shell recover [--yes]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes]

shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes]
shine app refresh <CATEGORY> [FILE] [--force] [--yes]
shine app recover [--yes]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes]
shine app artifact apply <APP_ID> [--yes]
shine app artifact remove <APP_ID> [--yes]
```

`--replace-managed` 会覆盖安装后被用户修改的受管内容。先使用 `shine info <TARGET> --diff` 检查差异。`app uninstall --force` 会删除被用户修改过的受管文件，执行前应加 `--dry-run` 预览。对于符合条件的静态 Copy，该强制删除会写入 journal，并把修改后的文件作为同目录 rollback material 暂存到 receipt commit；管理员静态 Copy 的创建、原地更新和移除通过 privileged write、move、mode 还原与 cleanup 使用同一 journaled transaction。JSON merge 的 install、原地 update、普通 uninstall 和强制 uninstall 也会按顶层 key ownership 写入 journal；其它安装策略仍使用原有 lifecycle 路径。

`shell install --dry-run` 会解析 metadata、部署来源、Bun 策略和计划中的命令入口，但不会提取或
快照预设、渲染模板、创建链接、写入 manifest 或修改 shell profile。

首次安装命令时，Shine 会在写入 Unix symlink、Unix Bun/live launcher 或 Windows
PowerShell/cmd 双 shim 之前记录 launcher creation journal。只有精确的 command receipt
持久化后才会清理 journal，shell profile 编辑发生在这之后。如果操作在此期间中断，后续修改型
Shell 命令会停止并提示恢复。运行 `shine shell recover` 可审阅独立的 recovery Plan。没有匹配
receipt 时，它只移除 target 或内容 hash 与 mode 仍精确匹配的 transaction-created launcher
resource；路径发生变化会阻塞恢复并保留现状。精确 receipt 已持久化时，恢复保留 launcher，只
清理 stale journal。确认默认是 No；非交互终端必须传入 `--yes`。

install 与 upgrade 更新 launcher 时，如果旧 command receipt 和所有 launcher resource 仍精确匹配，
也会写入 journal。发生变化的旧资源会先移到同目录的规范 `.shine.rollback` 路径。新 receipt
持久化前，恢复只还原精确匹配的旧资源；receipt commit 后，恢复保留精确 replacement，仅移除未修改
的 rollback material。replacement、rollback resource 或 receipt 发生冲突都会阻塞恢复。foreign
或已经被修改的 launcher 不会继承这套 rollback proof。

已批准的 uninstall 只会在旧 receipt 与重建出的每个 launcher resource 仍精确匹配时记录 launcher
removal journal。每个 Unix launcher 或 Windows shim 都会在 receipt 删除前移到同目录
`.shine.rollback`。receipt 删除后，必须另有持久化的 journal marker 确认 commit，才能清理
rollback。如果 receipt 已删除但 marker 尚未写入，`shine shell recover` 会先重建旧 receipt，再
还原精确资源。marker 持久化后，恢复会保留已完成的卸载，只移除未修改的 rollback material。
launcher、rollback 路径或 receipt 冲突发生变化时，恢复会阻塞并保留现场。

不使用 `--dry-run` 时，App 与 Shell 生命周期 mutation、App refresh 和 artifact apply/remove
都会先显示绑定快照的安全 Plan，并以默认 No 询问一次。`--yes` 仍会完整显示并重新校验 Plan，
只跳过提示；重定向输出等非交互执行必须传入该参数。在提供 dry-run 的命令中，`--yes` 与
`--dry-run` 互斥；dry-run 保持原有预览格式，不是已批准 Plan。

`app refresh` 只处理 manifest 已跟踪的生成式文件；失败时保留上次成功内容。`app artifact apply/remove` 显式运行预设声明的外部集成脚本，Shine 不会把 apply 隐式作为普通安装或升级的一部分。

如果受支持的 App creation、原地静态 Copy update，或未修改静态 Copy 的普通
removal 在 operation journal 写入后中断，之后需要安全
Plan 的 App mutation 命令会停止并
提示恢复，不会隐式修改这段中断状态；只读检查也不会恢复或丢弃 journal。运行
`shine app recover` 可以审阅独立的 recovery Plan。中断后被用户修改的文件会保留；对于
backup-aware creation，只有 destination 与固定 backup 仍匹配 journal 绑定的原始/目标 fingerprint
时才恢复 backup。原地 managed update 会把前一个受管文件临时移动到
`<name>.shine.rollback`；只有它仍匹配 journal 绑定的旧 fingerprint 时，恢复才会还原或移除它。
普通 removal 中，精确的旧 receipt 仍存在时会还原未修改的 rollback material；receipt 移除持久化后
还必须有 journal 中对应的 commit 状态，才会移除该未修改 material。receipt 缺失但没有这个状态时
恢复会重建旧 receipt，并还原未修改的文件。两种可恢复情况都绑定原 mode。
对于需要恢复 backup 的 removal，Shine 先把受管文件移到 `.shine.rollback`，再把 `.shine.bak` 移到
destination。receipt commit 前，恢复只会反转这三个路径的精确安全状态，同时恢复受管 destination
与 persistent backup；commit 后则保留 destination 中精确匹配的用户原文件，只移除未修改的受管
rollback material。两个文件的 mode 与内容 fingerprint 都必须与 journal 一致。
强制移除被用户修改过的静态 Copy 会使用独立 action：receipt commit 前的恢复会还原
精确的修改后文件并反转可选 backup restoration；commit 后的恢复会保留已完成卸载，只移除与所
捕获修改后 mode/hash 匹配的 rollback material。
JSON merge recovery 只把精确的完整 rollback 文件用作旧声明 key 值的来源。它会在当前 object
中还原或移除这些 key，不会替换中断后发生变化的其它值。uninstall receipt commit 后，它会保留
用户所有的当前 object，只移除精确匹配的 rollback material。
管理员静态 Copy 的 recovery 仅在精确恢复状态需要 write、move、remove 或改变受保护路径 mode 时
包含 administrator permission。Shine 会在 recovery Plan 获批后请求授权；仅修复 receipt 或清理
stale journal 不会请求。
中断后的 rollback material 可能包含敏感受管配置。ownership receipt 已持久化时，Shine 保留受管
destination 与持久 backup，只清理 stale transaction state。恢复确认默认是 No；没有交互终端时
必须传入 `--yes`。journal 缺失或无效、action 不受支持，或 destination/backup/rollback 已被修改时，
命令返回非零且不执行 mutation。已有固定 backup 或 update rollback path 也会阻塞相应的受支持
Plan，不会被替换；removal rollback path 也遵循相同规则。

## 状态、更新与补全

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <app/CATEGORY|sys/ITEM>
shine trust grant <app/CATEGORY|sys/ITEM> [--yes]
shine trust list
shine trust revoke <app/CATEGORY|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

Trust enrollment 从当前不可变 Preset snapshot 推导范围。`--yes` 只用于非交互确认当前展示的
enrollment，不会批准之后的 lifecycle Plan。

`app info`、顶层 `info` 和 `update` 默认都不执行 App generator。无法静态确定动态预期内容时，
这些命令会醒目提示 generator 尚未评估，不会把已安装文件误报为最新。传入
`--run-generators` 后，Shine 会显式执行自动和手动 generator，在内存中应用 transform 并计算
状态或 `--diff`，但不会写入目标文件或 manifest。全局 `update --run-generators` 会评估所有
已安装 App 类别，定向 info/update 只评估选中的 App。外部 generator 仍需匹配当前代码与权限的
`shine trust grant`；某项评估失败时，其余 generator 仍会继续，最后统一报告不完整结果。

- `update --refresh-release` 跳过 24 小时版本检查缓存。`update` 默认复用 `shine list` 的
  Homebrew 风格分栏：交互终端横向排列，重定向输出则保持每行一个 target；末尾只提示
  一次 `shine upgrade`。App 文件与 Shell 命令都按类别折叠。`update --diff` 会改用纵向
  详细行并展开受影响的文件与命令；来源或目标迁移、新文件、部署元数据和命令入口刷新等
  结构性变更会逐字段显示，只有内容确实变化时才输出 unified diff。定向的
  `update <TARGET>` 使用相同明细。
  只有结构变化时，Shine 会分别指出缺失或不匹配的命令入口与缺失的 Shell manifest 记录，
  并显示 `content: unchanged`，而不是输出空 diff。
  定向的 `update <TARGET>` 本身已经显示详情，因此 `--diff` 只会把不带 target 的 update 从
  类别摘要切换为展开行。
- 内联 diff 要求两侧都是不含 NUL 字节的有效 UTF-8 文本，并且每侧不超过 256 KiB。
  二进制、无效 UTF-8 或更大的内容只显示字节数摘要，不会整段写入终端；`info --diff`
  使用相同保护。
- 为 `update` 指定 target 后仍可同时传入 `--verbose` 以兼容通用命令行调用，但定向输出本身
  已包含详细信息，因此不会增加更多条目。定向检查不会检查 Shine 版本，仍不能与
  `--refresh-release` 组合使用。
- `update/upgrade --pull` 会先同步 Git 管理的来源并重新加载配置。
- 无 target 的 `upgrade` 会一次展示 Shell、App 和已启用 managed Sys 的 Plan，只确认一次，
  并在写入前复核全部 Plan；它不再隐式修改 Sys profile 的启用状态或组合内容。
- `upgrade --prune-stale` 移除预设来源中已不存在的旧受管 app 文件。
- `upgrade` 默认逐项显示实际更新的 App 类别、Shell 类别或受管系统项，并按用户可见
  target 各计数一次；app 行会附带变更文件数。`--verbose` 会展开 app 文件和成功 hook 的
  输出，还会显示已是最新或跳过的项目，以及 snapshot、template、Bin Link 等 Shell
  部署细节。失败、冲突、用户修改警告和被拦截的 hook 无需 `--verbose` 也会显示。
- `shell info` 和顶层 `info` 可以检查尚未安装的预设；`list --available` 可按资源类型过滤。
- 默认的 list、update 与 upgrade 摘要使用类别级生命周期身份；`info`、`--diff` 与
  verbose 部署区段仍保留文件、命令、入口和 receipt 明细。

## 系统预设

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes]
shine sys profile enable <ITEM> [--dry-run] [--yes]
shine sys profile disable <ITEM> [--dry-run] [--yes]
shine sys apply [ITEM] [--dry-run] [--yes]
shine sys uninstall <ITEM> [--dry-run] [--yes]
```

位置参数 item、重复的 `--item` 与 `--preset` 三者互斥。执行变更前，`sys bootstrap` 会展示绑定
输入快照的安全 Plan，并以默认否请求确认。非交互环境使用 `--yes`；它仍会展示并重新验证
Plan，且不能与 `--dry-run` 同时使用。Bootstrap 只确保选中的软件存在，并启用其声明的 shell
集成；重复运行不会升级软件。`sys profile enable/disable` 使用同一套 Plan 批准契约，并且只修改
Shine 自己管理的集成内容。第三方软件升级请使用其包管理器或上游工具；独立受管系统项可通过
`shine upgrade sys/<ITEM>` 收敛到当前预设状态。

## 预设来源与定制

```text
shine preset new <app|shell|sys> [--force]
shine preset validate [PATH] [--format <text|json>]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset copy` 只把一个完整的内置预设复制到当前目录，适合创建局部 overlay；`preset export`
导出整套内置预设。外部 Shell 预设默认以快照方式运行，来源内容变更需通过
`shine upgrade` 应用；`--live` 只适合预设开发，令源内容在下一次调用时生效。

`preset validate` 接受预设仓库根目录、`app|shell|sys/<name>` 类别目录或其中的
`shine.toml`；默认检查当前目录。它会静态检查所有平台分支和引用文件，不读取当前激活的预设
来源、不初始化 Shine 配置、不检查更新、不联网，也不运行任何预设代码。输入或类别无效时退出码
为 1，warning 不会导致失败。JSON 输出固定使用 `schema_version: 1`，不含颜色，也不会在 JSON
文档之外输出说明文字。Git 管理来源的安全限制及完整流程见[自定义预设](../guides/custom-presets.md)。

## 环境变量与密钥

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env broker describe [--workspace <FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy <add|update> --name <NAME> --ssh-target <TARGET> [--project <PROJECT>] --workspace <FILE> [--remote-workspace <REMOTE_FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy diff <NAME> --workspace <FILE> --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy list
shine env broker policy info <NAME>
shine env broker policy remove <NAME>
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity list
```

`--with` 可重复使用，写成 `KEY=ALIAS` 可改变子进程看到的变量名。`--no-workspace` 只使用显式值和已有进程环境，不能与 `--workspace` 或 `--mode` 同时使用。`workspace init` 只接受 `--from-dotenv`，可先用 `--dry-run` 预览生成文件。`workspace export` 必须显式指定格式、mode 和输出路径；默认只导出合并后生效的普通值，添加 `--include-secrets` 才会解密并包含 secret，且不会混入当前进程变量。broker 策略必须用一个或多个 `--release` 选择密钥，或用 `--release-all-declared` 固化当前环境源声明的全部密钥；二者不能组合。Touch ID identity 只适用于 macOS，并依赖 `age-plugin-se`。

创建 broker 策略时，`--project` 用于保存便于识别的项目标签；`--remote-workspace` 要求远端
请求除了匹配 workspace 内容和其它策略字段外，还必须报告这个完全一致的绝对 workspace 路径。

`env proxy install` 在 `~/.shine/bin/` 创建同名 PATH shim，按规则仅向目标子进程注入 `--with` 指定的值；每个值优先读取 `<KEY>_SECRET`，否则读取 `<KEY>`。`disable` 保留 shim 但跳过解密和注入；项目规则需在当前目录或其祖先存在 `shine.config.toml`，并覆盖同名全局规则。`uninstall` 移除 Shine 管理的 shim 和用户级规则。

## 任务、本地服务与主题

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine serve install [--port <PORT>]
shine serve start [--port <PORT>]
shine serve status
shine serve uninstall
shine serve url <PATH> [--port <PORT>]

shine theme sync [--auto] [--quiet]
```

任务按参数数组保存并直接执行，不经过 shell。`--cwd` 将任务固定到指定工作目录；未设置时继续使用调用者的当前目录。`serve install` 在 macOS 使用 launchd、在 Linux 使用 systemd user unit、在 Windows 使用当前用户的计划任务；`start` 可在前台启动本地服务。

## SSH 会话、密钥代理与文件传输

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [--secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session]] [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine 自己的选项必须写在 SSH 目标之前。远端按需请求密钥时使用 `shine env run --secret-broker`，详见 [SSH 会话：密钥代理与文件传输](../guides/ssh-transfer.md#按需向远端命令提供密钥)。Windows 远端使用 `--remote-shell windows`，该模式仅提供 PowerShell 环境注入，不建立 `shine local` 传输通道，也不支持 secret broker。

## 程序安装与升级

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

`shine --version` 在稳定版显示 `shine 1.8.0 (<commit> <date>)`；preview 构建使用 `1.8.0-preview` 形式的版本标签。
