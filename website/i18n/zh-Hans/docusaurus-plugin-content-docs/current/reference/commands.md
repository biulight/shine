---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页适用于 Shine 2.3.0。使用 `shine <COMMAND> --help` 查看当前安装版本支持的全部选项。完整操作流程
请阅读对应指南；本页只提供便于查找的命令索引。

## 目标名称

在脚本和文档中使用完整目标：

- 应用配置使用 `app/<category>`；
- Shell 预设使用 `shell/<category>` 或 `shell/<category>/<command>`；
- 受管系统项目使用 `sys/<item>`。

当 App 与 Shell 类别名称不会混淆时，也可以省略类型。单独的 Shell 命令名只用于查看信息。

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
| `shine serve ...` | 提供 `~/.shine/http/` 下的本地资源 |
| `shine self ...` | 安装或升级 Shine 程序 |
| `shine state migrate` | 预览或应用受支持的旧状态迁移 |
| `shine trust ...` | 审阅外部预设代码的目标级信任 |
| `shine completions ...` | 生成或安装 Shell 补全 |
| `shine theme sync` | 输出终端主题环境变量 |

所有命令都接受全局选项 `--config-dir <PATH>`。

Security Plan 默认显示用户文件的具体路径，并按用途汇总安装产物、Shine 内部维护及关联的备份/回滚文件，注明访问类型和路径数量。安装、升级、卸载、恢复、App refresh/artifact 和 Sys bootstrap/profile/apply 的审批都支持 `--verbose`，可展开已显示审阅范围中的文件路径、步骤、快照标识和诊断。

文件系统权限路径使用常见形式：绝对路径显示为 `/etc/docker/daemon.json`，用户主目录下的路径显示为 `~/.zshrc`，已配置的 Shine 状态目录下显示为 `Shine state/...`，平台应用数据目录下显示为 `App data/...`。Windows 盘符和网络共享路径保留根路径。这些显示标签不会扩大所列权限。
确认提示为 `Apply this Plan?`；只有列出外部/overlay 代码的 Plan 才会同时询问是否允许本次操作使用这些代码，并说明不会保存持久信任。

Shell 命令安装按命令数合并，Shell 集成单独显示；内部状态与恢复文件合并统计路径数量，用户文件的备份关联仍会明确提示。`--verbose` 保留逐项明细。

已确认无变化的 App 文件及受管 Sys 项目不会贡献本次操作权限。默认 `upgrade` 审阅不显示没有动作、所需权限、代码边界、阻塞或特殊诊断的范围，也不列出普通的未变化步骤和日常升级跳过的手动 App 生成器。`upgrade --verbose` 同样省略这些无关范围和步骤，并展开其余步骤、具体权限、快照标识、诊断码和指纹。需要查看所有计划范围和步骤时，可用 `upgrade --verbose --full-plan`；`--full-plan` 必须与 `--verbose` 一起使用。完整 Plan 仍参与审批绑定，审阅后发生变化必须重新规划。实际触发的生成器、hook 和共享事务保留各自权限。正常事务码及生命周期快照标识仅在 `--verbose` 中显示；保留、阻塞及其它诊断提示仍明确展示。

默认升级审阅将常规 App 和 Shell 预设缓存写入归入内部维护。这些来源副本不代表应用配置或命令有更新。仅涉及缓存的 App 工作归入内部维护，不再显示于 `App Configs`；旧来源消失的保留提示单独显示在“警告”中。实际 App 变更、代码执行、阻塞、冲突及无法安全归组的影响仍明确展示。使用 `upgrade --verbose` 可查看受影响的分类、各个缓存步骤和具体权限。完整 Plan 仍参与审批绑定。

确认后，`upgrade --verbose` 的执行报告也省略未变化的 App 文件、已安装的受管 Sys 项目及 Shell 已安装分类总数，但仍详细显示实际更新、冲突、警告和失败。`upgrade --verbose --full-plan` 会包含未变化的执行记录。普通模式下全部未变化时报告 `Nothing to upgrade.`。

Shell 升级会在审阅前比较 Shine 管理的 profile 和已配置的 Shell 启动文件。只有确实需要协调的文件才列在 `Shell integration (internal)` 下；未变化的启动文件及其权限不会显示。`shell/profile` 是内部 Plan 标识，不是 Shell 预设类别。

恢复操作、阻塞的计划以及无法可靠归类的权限仍显示具体路径。代码信任、管理员权限和其它能力提示不会被省略。摘要只改变展示方式，不会授予整个 `~/.shine` 目录的访问权限，也不是脚本沙箱；`--dry-run` 的预览行为保持不变。

## Shell 预设

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes] [--verbose]
shine shell recover [--yes] [--verbose]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes] [--verbose]
```

- 使用 `--dry-run` 预览安装或卸载。
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
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes] [--verbose]
shine app artifact apply <APP_ID> [--yes] [--verbose]
shine app artifact remove <APP_ID> [--yes] [--verbose]
```

- `app info` 和 `update` 只有在传入 `--run-generators` 时才会运行生成器。
- `app refresh` 显式刷新生成文件；`--force` 允许替换用户修改过的受管目标。安装收据必须属于
  该类别下的同一个源文件；`--force` 不能刷新由其他来源管理的目标。
- `app uninstall --force` 可能删除用户修改过的受管内容，务必先使用 `--dry-run` 预览。
- 受到保护的 App 文件会说明与安装记录不一致，随后集中显示内容与记录保留的结果。
  强制预览指引按类别汇总，说明删除文件、按备份是否存在恢复配置，或移除受管 JSON 键的影响。
  使用 `--force --dry-run` 或 `--verbose` 查看已记录备份路径；无关 JSON 键仍保留。
- App 操作中断并阻塞后续变更时，使用 `shine app recover`。

参见 [管理应用配置](../guides/app-presets.md)。

已安装的 App 类别或单个来源文件被删除、改名，或切换来源后不可用时，安装内容仍受管理。
`list` 继续显示该类别；`list`、`update` 和 `upgrade` 会说明来源缺失并给出卸载预览命令。
升级默认保留这些内容。

先使用 `shine app uninstall <OLD_CATEGORY> --dry-run`，再使用
`shine app uninstall <OLD_CATEGORY>`，即可在 Preset 不可用时按原安装记录卸载。
卸载只选择该类别的安装记录，不会认领其他类别在同一目标路径的记录。
复制安装会移除未修改的受管内容，并恢复仍存在的已记录备份；JSON 合并安装只移除安装记录
拥有的键，保留无关键。用户修改过的受管内容及其记录默认保留，只有显式 `--force` 才可覆盖。
目标不存在时，卸载清理安装记录，并保留已记录备份。预览不改变文件、备份或记录，
会说明用户修改及已记录备份的恢复行为。来源代码缺失时无法运行其 teardown。

App 和 Shell 卸载会区分 `Uninstall complete`（已完成）、`Uninstall incomplete`（未完成）
和 `Uninstall preview`（预览）。实际卸载留下受保护内容或冲突资源，或发生执行失败时，
即使部分项目已完成，也会返回非零退出码。请检查各资源结果：非零退出码不会撤销已完成的删除。
成功生成的预览即使提示保护也返回零；预览本身失败仍返回非零。
默认卸载计划隐藏已知的无变化缓存步骤和空权限段；`--verbose` 保留完整审查。
完整展示未完成原因后，命令仍以非零退出，但不重复输出泛化的 `Error` 行；其他错误仍正常显示。

## 状态、更新、信任与补全

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--full-plan] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine trust grant <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM> [--development] [--yes]
shine trust list [--verbose]
shine trust revoke <preset|app/CATEGORY|shell/CATEGORY/COMMAND|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

`update` 只读检查；`upgrade` 显示计划，并在存在动作时等待确认。全部未变化时只报告 `Nothing to upgrade.`，不要求确认。只有审阅过同一范围后才应使用 `--yes`。
`--pull` 会先更新符合条件的 Git 预设来源；`--prune-stale` 允许删除预设中已经不存在且未被修改的受管项。
`trust grant --development` 会让所显示本地来源中的代码修改继续受信任，但 target、capability、
来源目录与来源层必须保持不变。这是来源级长期授权，不代表持续代码审阅。
`trust list` 会将共享同一安全范围的 grant 合并为紧凑行，并对照当前 Preset 检查状态；`--verbose`
会展开 capability、开发来源标签与复查提示。

交互式生命周期确认可仅授权本次外部代码，不保存 grant；自动化与 `--yes` 必须已有 trust，
Shell live 必须使用 Development trust。

来源缺失、用户修改、外部命令冲突、env/admin 契约缺失或外部代码尚未信任时，Shine 会提示需要处理，
不会静默覆盖。请恢复来源或按终端给出的命令处理。

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

使用 `bootstrap` 确保选中的软件和 Shell 集成存在；使用 `apply` 与 `uninstall` 管理可撤销的系统配置。
这些操作可能在计划获批后请求管理员权限。`--force-profile` 可能替换冲突的配置文件内容，请先查看
dry run。

托管配置预览（`sys apply --dry-run` 和 `sys uninstall --dry-run`）会在标题标记 `(dry-run)`，
并用 `→ would update` 或 `→ would remove` 表示计划中的变更。这些行不表示已经修改配置；
实际卸载成功后显示 `✓ removed`。

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

发布预设前使用 `validate`、`lint`、`plan` 和 `test`。这些命令只检查创作输入，不会安装预设。
`migrate --dry-run` 用于预览旧 metadata 迁移，实际应用前需要审阅并确认。Git 管理的 overlay 是
可丢弃镜像，应在上游 checkout 中编辑，而不是修改镜像。
新模板不会生成空表或推测性能力清单。`--unrestricted` 会添加可选的 unrestricted 作者说明，
但无论是否使用，所有任意代码都未隔离。

参见 [自定义预设](../guides/custom-presets.md)与
[将系统预设迁移到 v2](../guides/sys-preset-v2-migration.md)。

## 环境变量与密钥

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity init --phone [--recipient-type <tag|phone>] [--label <LABEL>] [--transport <auto|adb|qr>] [--adb-serial <SERIAL>]
shine env secret identity list
```

`env list --reveal`、`env get`、`env secret decrypt`、带 `--include-secrets` 的导出，以及 `env run`
启动的子进程都可能接触明文。只在可信终端运行，也不要把密钥直接写入命令参数或文档。

Broker policy 命令及完整远程流程见
[SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。本地变量、工作区加密、命令包装和 hybrid
后端见 [管理环境变量与密钥](../guides/environment.md)。

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

任务会运行保存的命令，并可使用保存的工作目录。本地服务会发布 `~/.shine/http/` 下的文件，不要在该
目录放置密钥。

参见 [任务与本地服务](../guides/tasks-and-serve.md)和
[同步终端主题](../guides/terminal-theme-sync.md)。

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
```

`stable` 跟随正式发布，`preview` 跟随会被持续替换的预览构建。参见
[安装与升级](../installation.md)。
