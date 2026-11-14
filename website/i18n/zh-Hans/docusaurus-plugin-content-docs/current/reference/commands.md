---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页对应当前开发版（Next）。使用 `shine <COMMAND> --help` 查看已安装版本支持的选项。完整操作流程
请阅读对应指南；本页只提供便于查找的命令索引。

命令失败信息写入 stderr。设置 `NO_COLOR=1` 可禁用彩色输出。

## 目标名称

在脚本和文档中使用完整目标：

- 应用配置使用 `app/<category>`；
- Shell 预设使用 `shell/<category>` 或 `shell/<category>/<command>`；
- 受管系统项目使用 `sys/<item>`。

当 App 与 Shell 类别名称不会混淆时，也可以省略类型。单独的 Shell 命令名只用于查看信息。

目标名称按原值匹配。名称包含首尾空白时，请用引号完整引用；Shine 不会去掉这些空白。
完全由空白组成的目标会被拒绝。

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

## 顶层命令

| 命令 | 用途 |
| --- | --- |
| `shine init [--yes]` | 为当前项目创建 `shine.config.toml` |
| `shine list [--available [KIND]]` | 查看已安装资源或浏览 `app`、`shell`、`sys` 目录 |
| `shine info <TARGET> [--diff] [--verbose]` | 查看可用或已安装目标的详情 |
| `shine install <TARGET> [--verbose]` | 安装或修复 App/Shell 目标 |
| `shine uninstall <TARGET> [--verbose]` | 卸载 App/Shell 目标 |
| `shine update [TARGET]` | 检查受管内容与 Shine 版本，不应用改动 |
| `shine upgrade [TARGET]` | 应用全部或指定的受管更新 |
| `shine shell ...` / `shine app ...` / `shine sys ...` | 使用资源类型专属操作 |
| `shine preset ...` | 创建、校验和管理预设来源 |
| `shine env ...` | 管理变量、工作区环境、命令代理和密钥 |
| `shine ssh ...` / `shine local ...` | 建立 SSH 会话与传输文件 |
| `shine task ...` / `shine run <NAME>` | 保存和运行个人命令 |
| `shine self ...` | 安装或升级 Shine 程序 |
| `shine state migrate` | 预览或应用受支持的旧状态迁移 |
| `shine state check` | 立即检查需要手动处理的维护事项，不执行修改 |
| `shine clear serve` | 停止并移除已退役的 HTTP 用户服务注册 |
| `shine trust ...` | 审阅外部预设代码的目标级信任 |
| `shine completions ...` | 生成或安装 Shell 补全 |
| `shine theme sync` | 输出终端主题环境变量 |

所有命令都接受全局选项 `--config-dir <PATH>`。

目标名称含空格或 shell 特殊字符时，请加引号。名称以 `-` 开头时，将选项放在 `--` 前、
名称放在其后，例如 `shine app uninstall --dry-run -- -demo`。
终端建议的命令在 Unix 上使用 POSIX shell 语法，在 Windows 上使用 PowerShell 语法。

确认变更前，请审阅受影响的文件、权限、代码来源和执行步骤。添加 `--verbose` 查看详情；
升级时可同时添加 `--full-plan`，包括未变化的项目。审阅后相关文件或设置发生变化时，
请重新审阅计划。

交互确认可以仅授权本次列出的外部代码，不保存长期信任。通过自动化或 `--yes` 执行外部代码时，
必须已有信任授权。
`--yes` 只跳过提示，不跳过权限和执行检查，也不能与 `--dry-run` 同时使用。预设脚本没有沙箱隔离。

## Shell 预设

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine shell recover [--yes] [--verbose]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--yes] [--verbose]
```

- 使用 `--dry-run` 预览安装或卸载。
- `shell install --dry-run` 只列出链接候选，并非安全计划；它不评估归属冲突、
  `--replace-managed` 的替换效果、信任、权限或 profile 变更，`--verbose` 也不会扩展该预览。
  去掉 `--dry-run` 后，可在批准前审查计划。
- `shell list` 展示当前可用预设，包括 overlay 新增的命令和覆盖后的描述。
- `--replace-managed` 可能覆盖安装后被修改的受管内容，请先运行 `shine info <TARGET> --diff`。
- 如果 Shine 报告操作曾被中断，请运行 `shine shell recover`。发生过变化的文件会保留，可能需要人工处理。
- Shell 卸载遇到归属冲突时，会列出未删除的启动器路径。Shell 没有 `--force` 覆盖选项；
  请先检查这些文件，再决定是否移动或删除。冲突文件可能在安装记录清理后仍保留，输出会明确说明。

参见 [管理 Shell 预设](../guides/shell-presets.md)。

## 应用预设

```text
shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine app refresh <CATEGORY> [FILE] [--force] [--yes] [--verbose]
shine app recover [--yes] [--verbose]
shine app uninstall [CATEGORY] [--force] [--dry-run] [--yes] [--verbose]
shine app artifact apply <APP_ID> [--yes] [--verbose]
shine app artifact remove <APP_ID> [--yes] [--verbose]
```

- `app info` 和 `update` 只有在传入 `--run-generators` 时才会运行生成器。
- `app refresh` 显式刷新生成文件；`--force` 允许替换用户修改过的受管目标。目标必须已由
  该类别下的同一个源文件管理；`--force` 不能接管其他来源的目标。
  汇总会区分 `user-modified (kept)`（用户修改已保留）和执行失败；两者都会使刷新结果为未完成，
  并返回非零退出码，其他选中的文件仍可能刷新成功。
- `app uninstall --force` 可能删除用户修改过的受管内容，务必先使用 `--dry-run` 预览。
- App 操作中断并阻塞后续变更时，使用 `shine app recover`。

参见 [管理应用配置](../guides/app-presets.md)。

预设来源消失后，已安装的 App 文件仍会保留。先用
`shine app uninstall <OLD_CATEGORY> --dry-run` 预览，再去掉 `--dry-run` 执行卸载。
类别名称冲突时，使用 `app/` 或 `shell/` 前缀。

卸载默认保留用户修改过的文件，删除未修改的受管文件并恢复可用备份；JSON 合并卸载只移除
受管键。`--force` 允许删除本地修改。来源代码缺失时无法运行 teardown 脚本。
恢复与清理细节见 App 指南。

App 或 Shell 卸载未完成时，即使部分删除已成功，也会返回非零退出码；已完成的删除不会撤销。
请检查剩余冲突后再重试。`--dry-run` 成功只代表预览完成，不代表所有资源都可以删除。

## 状态、更新、信任与补全

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--full-plan] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine state check
shine trust inspect <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine trust grant <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM> [--development] [--yes]
shine trust list [--verbose]
shine trust revoke <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

动态补全会列出当前预设来源及其生效 overlay 中的候选。
按 Tab 补全不会执行预设代码或授予信任。

`info` 查看文件但不修复它们。`state migrate --dry-run` 预览受支持的旧状态迁移；
如果配置校验失败，请修正提示的文件后重试。

版本检查失败时，`update` 仍会报告配置更新。恢复连接后，可用 `shine update --refresh-release`
重试版本检查；该选项不能与目标名称同时使用。

`update` 默认不执行生成器；添加 `--run-generators` 可求值并检查生成结果。完全来自内置预设的
类别无需额外授权；类别中含外部或 overlay 文件时，需要匹配的信任授权。请先审阅报告的来源再授权。

评估失败或未完成时，无法据此确认已安装内容是最新的。即使只检查一个文件，同一 App 类别中
其他生成器失败也可能使命令返回退出码 1。使用 `--verbose` 查看诊断，或通过 `preset lint`
[检查预设源码](../guides/custom-presets.md#执行前检查-app-代码)。

外部 Shell 预设使用 `snapshot` 模式时，`update --diff` 也会显示类别共享文件的变化。
`content: unchanged` 仅说明命令脚本未变，不代表辅助文件或元数据无需更新。
`update <TARGET>` 默认显示这些详情。差异用于审阅，不能直接作为补丁；仅换行格式变化也可能需要更新。

`update --verbose` 也会列出暂无更新的受管系统配置。`up-to-date (recorded)` 表示期望配置与
Shine 的安装记录一致，并非实时系统检查。请处理来源缺失或生成器未检查等提示，再判断是否全部为最新。

已安装的 App 类别或文件从预设来源中消失后，`update --verbose` 仍会报告保留的文件，并提供
卸载预览命令；检查本身不会删除它们。

`update` 只检查、不应用改动。`upgrade` 在需要修改时请求批准；没有待处理工作或未解决的检查时，
报告 `Nothing to upgrade.`。即使命令内容未变，配置或 Shell 集成仍可能需要更新。
使用 `--yes` 前请先审阅计划。

| 选项 | 作用 |
| --- | --- |
| `--pull` | 检查或升级前，先更新符合条件的 Git 预设来源 |
| `--prune-stale` | 允许删除预设中已不存在且未被修改的受管 App 文件 |
| `trust grant --development` | 授权所显示本地来源中未来的代码修改 |

Shell live 部署需要开发信任。授权仅覆盖批准的目标、能力及来源目录与来源层，不代表持续审阅
未来代码。使用 `trust list` 查看授权，添加 `--verbose` 查看详情；查看列表不会续授信任。
详见[信任与能力声明](../guides/custom-presets.md#声明权限)。

来源缺失、用户修改、命令入口冲突、缺少必需输入或管理员授权，以及外部代码尚未信任时，
请按报告的提示处理；这些情况不会授予覆盖文件的权限。

## 系统预设

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys recover [--yes] [--verbose]
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes] [--verbose]
shine sys profile enable <ITEM> [--dry-run] [--yes] [--verbose]
shine sys profile disable <ITEM> [--dry-run] [--yes] [--verbose]
shine sys apply [ITEM] [--dry-run] [--yes] [--verbose]
shine sys uninstall <ITEM> [--dry-run] [--yes] [--verbose]
```

`sys list --all` 将其他操作系统标记为 `catalog only`（仅供浏览），只为当前系统显示 apply 提示。
`sys info`、`bootstrap`、`apply` 和 `uninstall` 始终按当前系统查找项目；其他系统存在同名项目时
也不会切换查找范围。

使用 `bootstrap` 确保选中的软件和 Shell 集成存在；使用 `apply` 与 `uninstall` 管理可撤销的系统配置。
这些操作可能在计划获批后请求管理员权限。`--force-profile` 可能替换冲突的配置文件内容，请先查看
dry run。

应用或删除系统配置前，可用 `--dry-run` 预览变更。它不能与 `--yes` 同时使用。
`--yes` 用于非交互确认，只跳过提示，不会跳过计划展示、权限检查或执行前复核。
交互确认可以仅授权本次列出的外部代码，不保存长期信任；使用 `--yes` 执行外部代码前，
必须已有相应的[信任授权](../guides/custom-presets.md)。

`bootstrap` 的位置参数按输入顺序选择项目，并忽略重复项；也可以重复使用 `--item`，例如
`shine sys bootstrap --item rust --item mise --yes`。位置参数、`--item` 和 `--preset` 不能组合。
非交互环境下，未指定项目或配置组合时使用预设的默认配置组合。
项目 ID 以 `-` 开头时，将选项放在 `--` 前、名称放在其后，例如
`shine sys apply --dry-run -- -custom`。

对于受管系统配置，`shine update` 对比期望配置与安装记录，`shine upgrade` 还会检查实际系统资源。
Windows 的 split-DNS 配置与记录和期望配置一致时，无需更新或管理员权限。
若只想升级 Shell 预设，可使用 `shine upgrade shell/<CATEGORY>`。

参见 [初始化与管理系统](../guides/system-init.md)。

## 预设创作与来源

```text
shine preset new <app|shell|sys> [--unrestricted] [--force]
shine preset schema [--format <text|json>]
shine preset validate [PATH] [--format <text|json>]
shine preset lint [PATH] [--format <text|json>] [--deny-warnings]
shine preset plan <CATEGORY> --platform <macos|linux|windows> [--format <text|json>]
shine preset test <CATEGORY> [--format <text|json>]
shine preset pack <CATEGORY> --output <FILE> [--force] [--format <text|json>]
shine preset migrate [PATH] [--dry-run] [--yes] [--format <text|json>]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset overlay info` 展示实际生效的 overlay；项目的本地 overlay 优先于继承的 Git overlay。
存在项目配置时，仅设置 `SHINE_CONFIG_DIR` 不会覆盖 `preset link` 选定的目录；
`SHINE_PRESETS` 仍具有更高优先级。`preset unlink` 移除保存的设置，但环境覆盖和继承设置
仍然适用，因此下次运行可能继续使用外部预设目录。
发布预设前使用 `validate`、`lint`、`plan` 和 `test`，这些命令检查预设但不会安装它。
先用 `migrate --dry-run` 预览元数据变更，审阅并确认后再应用。Git 管理的 overlay 应在上游
checkout 中迁移、验证并提交，推送后在各使用设备上运行 `shine preset pull`。

`--unrestricted` 会为新模板添加可选的作者说明，但不会隔离脚本执行，也不会授予信任或执行权限。

参见 [自定义预设](../guides/custom-presets.md)与
[将系统预设迁移到 v2](../guides/sys-preset-v2-migration.md)。

## 环境变量与密钥

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--global|--project|--overlay] [--env-file]
shine env get <KEY>
shine env explain <KEY> [--reveal]
shine env delete <KEY> [--global|--project|--overlay] [--env-file]
shine env run [--workspace <FILE>] [--mode <MODE>] [--file-group <NAME>]... [--dry-run] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env workspace export --format files [--workspace <FILE>] --mode <MODE> --file-group <NAME>... [--secret-broker] [--dry-run]
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--global|--project|--overlay] [--env-file]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]...
shine env secret seal --file-group <NAME>... [--workspace <FILE>] [--mode <MODE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]... [--remove-plaintext] [--dry-run]
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity init --phone [--recipient-type <tag|phone>] [--label <LABEL>] [--transport <auto|adb|qr>] [--adb-serial <SERIAL>]
shine env secret identity list
```

范围参数互斥；省略时选择当前项目，否则全局。`--env-file` 选择该范围的 `shine.env.toml`；
`--overlay` 只允许手动目录。变量被更高优先级来源覆盖时，请明确选择要修改的位置，并在完成后
核对生效来源。`set`、`delete` 与 `secret encrypt` 不再支持旧的 `--force`，请改用范围与文件参数。

`env explain --reveal`、`env list --reveal`、`env get`、`env secret decrypt`、带 `--include-secrets` 的导出，以及 `env run`
启动的子进程都可能接触明文。只在可信终端运行，也不要把密钥直接写入命令参数或文档。

Broker policy 命令及完整远程流程见
[SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。本地变量、工作区加密、命令包装和 hybrid
后端见 [管理环境变量与密钥](../guides/environment.md)。

文件组的配置、封存、持久导出、临时执行和清理边界见[临时机密文件](../guides/environment.md#workspace-secret-files)。

## 任务与主题

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine theme sync [--auto] [--quiet]
```

任务会运行保存的命令，并可使用保存的工作目录。

参见 [任务与应用构建产物](../guides/tasks-and-serve.md)和
[同步终端主题](../guides/terminal-theme-sync.md)。

## 清理

### 维护提醒

Shine 可在普通交互式使用时提醒维护事项。检查不会删除服务或文件。
需要立即检查时，包括脚本调用或配置无法加载时，可运行：

```text
shine state check
```

非交互调用会跳过自动提醒，部分提醒可能在后续调用时出现。
检查未完成时，请解决提示中的问题，再运行 `shine state check`。

### 移除旧 HTTP 服务

```text
shine clear serve
```

`clear` 必须指定目标；单独运行不会清理所有内容。`clear serve` 会停止并移除旧版用户服务，
无需加载 Shine 配置，也不会删除已发布的文件。成功后重复清理也是安全的；系统服务管理器
报错时会报告失败。整个 `shine serve` 命令已移除。参见
[升级与迁移](../guides/upgrading.md#清理旧版本的本地-http-服务)。

版本化运行时状态迁移仍使用 `shine state migrate`。

## SSH 与文件传输

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session] <HOST>
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine 的转发与 broker 选项必须写在 SSH 目标之前。直接转发密钥会让远程会话接触明文。文件传输
仅适用于 POSIX 远端；覆盖现有内容前先使用 `--dry-run`。

参见 [SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。

## 安装和升级 Shine

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
shine self uninstall [--dry-run | --yes]
```

`stable` 跟随正式发布，`preview` 跟随会被持续替换的预览构建。参见
[安装与升级](../installation.md)。
