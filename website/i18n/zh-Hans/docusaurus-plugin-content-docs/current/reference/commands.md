---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页适用于 Shine 2.5.0。使用 `shine <COMMAND> --help` 查看当前安装版本支持的全部选项。完整操作流程
请阅读对应指南；本页只提供便于查找的命令索引。

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

`shine init` 会转义目录确认提示和结果路径中的终端控制字符；确认流程和项目配置位置保持不变。

App 安装／修复、upgrade、artifact、refresh、卸载、Shell 修复及信任提示中的命令，会对含空格或 Shell
特殊字符的名称加引号，以保留原始参数边界。Unix 上请在 POSIX shell 中使用这些提示，Windows
上请使用 PowerShell。
位置参数名称以 `-` 开头时，这些提示会把选项放在 `--` 前、名称放在其后，
例如 `shine app uninstall --dry-run -- -demo`。

Security Plan 文本、App 文件结果、refresh 摘要、升级标题及重启提示，
会将终端控制字符显示为转义文本。原始目标、审批、文件操作和退出状态保持不变。分类名含控制字符时，
App artifact 和来源缺失提示不提供可复制命令，请使用原始分类参数。
目标解析错误也会转义输入的名称，匹配和拒绝规则保持不变。

Security Plan 默认显示用户文件的具体路径，并按用途汇总安装产物、Shine 内部维护及关联的备份/回滚文件，注明访问类型和路径数量。安装、升级、卸载、恢复、App refresh/artifact 和 Sys bootstrap/profile/apply 的审批都支持 `--verbose`，可展开已显示审阅范围中的文件路径、步骤、快照标识和诊断。

文件系统权限路径使用常见形式：绝对路径显示为 `/etc/docker/daemon.json`，用户主目录下的路径显示为 `~/.zshrc`，已配置的 Shine 状态目录下显示为 `Shine state/...`，平台应用数据目录下显示为 `App data/...`。Windows 盘符和网络共享路径保留根路径。这些显示标签不会扩大所列权限。
确认提示为 `Apply this Plan?`；只有列出外部/overlay 代码的 Plan 才会同时询问是否允许本次操作使用这些代码，并说明不会保存持久信任。

Shell 命令安装按命令数合并，Shell 集成单独显示；内部状态与恢复文件合并统计路径数量，用户文件的备份关联仍会明确提示。`--verbose` 保留逐项明细。

已确认无变化的 App 文件及受管 Sys 项目不会贡献本次操作权限。默认 `upgrade` 审阅不显示没有动作、所需权限、代码边界、阻塞或特殊诊断的范围，也不列出普通的未变化步骤和日常升级跳过的手动 App 生成器。`upgrade --verbose` 同样省略这些无关范围和步骤，并展开其余步骤、具体权限、快照标识、诊断码和指纹。需要查看所有计划范围和步骤时，可用 `upgrade --verbose --full-plan`；`--full-plan` 必须与 `--verbose` 一起使用。完整 Plan 仍参与审批绑定，审阅后发生变化必须重新规划。实际触发的生成器、hook 和共享事务保留各自权限。正常事务码及生命周期快照标识仅在 `--verbose` 中显示；保留、阻塞及其它诊断提示仍明确展示。

默认升级审阅将常规 App 和 Shell 预设缓存写入归入内部维护。这些来源副本不代表应用配置或命令有更新。仅涉及缓存的 App 工作归入内部维护，不再显示于 `App Configs`；旧来源消失的保留提示单独显示在“警告”中。实际 App 变更、代码执行、阻塞、冲突及无法安全归组的影响仍明确展示。使用 `upgrade --verbose` 可查看受影响的分类、各个缓存步骤和具体权限。完整 Plan 仍参与审批绑定。

确认后，`upgrade --verbose` 的执行报告也省略未变化的 App 文件、已安装的受管 Sys 项目及 Shell 已安装分类总数，但仍详细显示实际更新、冲突、警告和失败。`upgrade --verbose --full-plan` 会包含未变化的执行记录。普通模式下全部未变化时报告 `Nothing to upgrade.`。
定向升级与全量配置升级都会将用户修改过的受管 Sys 文件计入保留项。
若配置项目失败，末尾汇总显示 `Upgrade incomplete` 和失败数量，命令仍以非零状态退出。
非致命的 hook 警告维持现有行为。

Shell 升级会在审阅前比较 Shine 管理的 profile 和已配置的 Shell 启动文件。只有确实需要协调的文件才列在 `Shell integration (internal)` 下；未变化的启动文件及其权限不会显示。`shell/profile` 是内部 Plan 标识，不是 Shell 预设类别。

恢复操作、阻塞的计划以及无法可靠归类的权限仍显示具体路径。代码信任、管理员权限和其它能力提示不会被省略。摘要只改变展示方式，不会授予整个 `~/.shine` 目录的访问权限，也不是脚本沙箱；`--dry-run` 的预览行为保持不变。

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
- `shell list` 和 `shell info` 会转义名称、来源路径及描述行中的控制字符，保留注释段落间的
  空行。目标含控制字符时，安装或修复提示会要求使用原始目标参数，不提供可复制命令。
- `shell info` 建议的安装和修复命令会为目标名称加上必要的引用，包括含空格或引号的名称；
  Unix 使用 POSIX shell 语法，Windows 使用 PowerShell 语法。
- `--replace-managed` 可能覆盖安装后被修改的受管内容，请先运行 `shine info <TARGET> --diff`。
- 如果 Shine 报告操作曾被中断，请运行 `shine shell recover`。发生过变化的文件会保留，可能需要人工处理。
  结果按需要回滚或清理的 Shell 资源动作计数；已提交操作的清理会保留其安装内容。

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
- `app list` 和 `app info` 会转义所显示名称、描述与路径中的控制字符。
  分类名含控制字符时，安装提示会要求使用原始目标参数，不提供可复制命令。
- 安装、升级和刷新时，生成器的诊断提示输出到 stderr；展示的 hook 标准输出提示仍输出到 stdout。
- `app refresh` 显式刷新生成文件；`--force` 允许替换用户修改过的受管目标。安装收据必须属于
  该类别下的同一个源文件；`--force` 不能刷新由其他来源管理的目标。
  汇总会区分 `user-modified (kept)`（用户修改已保留）和执行失败；两者都会使刷新结果为未完成，
  并返回非零退出码，其他选中的文件仍可能刷新成功。
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
顶层命令 `shine uninstall app/<OLD_CATEGORY>` 同样支持这种清理。裸分类名称也会考虑来源已丢失的
已安装目标；名称冲突时，请使用 `app/` 或 `shell/` 前缀。
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

`state migrate` 在清理或写入之前校验项目配置和工作区文件。如果 TOML 语法无效，
或工作区的 `env`/`env.encryption` 字段不是表，会报告文件位置并以退出码 1 结束，
不回显文件内容。修正文件后重试；`--dry-run` 使用相同的校验。迁移预览和完成提示会转义
显示路径中的控制字符，迁移目标保持不变。

版本缓存失败警告会转义错误文本中的控制字符。缓存写入仍为尽力而为，失败不会把成功的版本
检查变为失败。

如果 Shine 版本检查未能完成，`update` 会输出警告，并继续报告配置更新。
没有发现配置更新时，会明确说明版本检查未完成，不会笼统地报告没有更新。
仅版本检查失败仍不会导致非零退出码。

`info` 显示已安装 App/Shell 的详情时，会转义标签、描述及路径标题中的控制字符。
`info --verbose` 所展示的文件内容中的控制字符保持原样。
`info`、`app info` 和 `update` 显示内容差异时，会转义行内的控制字符
（例如将制表符显示为 `\t`），文件内容保持不变。

`info <TARGET>` 和 `update <TARGET>` 仅针对选中的 App 分类或文件提示生成器未评估，
查询 Shell 目标时不会建议评估生成器。显式传入 `--run-generators` 时，即使目标是单个文件，
仍会评估其 App 分类中的生成器。信任或评估失败仍以非零状态退出；评估未完成时不会得出
`No update available` 的结论。
这些失败提示的标签和详情会转义终端控制字符。更新路径变更行，以及 `update --diff` 和
`update --verbose --diff` 中的 App 目标路径也采用同样显示方式，不会更改文件或安装记录。

`update --verbose` 也会列出已安装且暂无更新的受管系统配置。
`up-to-date (recorded)` 表示期望配置与安装记录一致，并非实时系统检查；Preset 来源缺失会单独提示。
App 汇总分别说明生成器未评估、需要信任或评估失败，不会将这些状态归为用户修改。
这些生成器状态按分类计数，使用 `--diff` 时按文件计数，不表示生成器执行次数。

如果 App 类别或文件条目已从当前 Preset 中移除，`update --verbose`（包括同时使用 `--diff`）
仍会提示已保留的安装内容，并提供卸载预览命令。即使整个类别已不存在，这些检查也不会删除
文件或安装记录。

`update` 只读检查；`upgrade` 显示计划，并在存在动作时等待确认。全部未变化时只报告 `Nothing to upgrade.`，不要求确认。只有审阅过同一范围后才应使用 `--yes`。
`--pull` 会先更新符合条件的 Git 预设来源；`--prune-stale` 允许删除预设中已经不存在且未被修改的受管项。
`trust grant --development` 会让所显示本地来源中的代码修改继续受信任，但 target、capability、
来源目录与来源层必须保持不变。这是来源级长期授权，不代表持续代码审阅。
`trust list` 会将共享同一安全范围的 grant 合并为紧凑行，并对照当前 Preset 检查状态；`--verbose`
会展开 capability、开发来源标签与复查提示。复查命令会将含空格或 shell 特殊字符的完整 target
保留为一个参数；Unix 使用 POSIX shell，Windows 使用 PowerShell。列表检查不会续授 grant。
并发执行 `trust grant` 与 `trust revoke` 会保留对其他 target 的修改；另一条命令保存旧版 trust
存储时不会恢复已经完成的撤销。

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

`sys list --all` 将其他操作系统标记为 `catalog only`（仅供浏览），只为当前系统显示 apply 提示。
Bootstrap 预览也会转义 profile 名称、代理值和脚本路径中的控制字符；预览不会执行安装器。

目录 list/info 中的描述与条目标签一样，将终端控制字符显示为转义文本；存储的描述和条目选择不变。
`sys info`、`bootstrap`、`apply` 和 `uninstall` 始终按当前系统查找项目；其他系统存在同名项目时
也不会切换查找范围。

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

`preset overlay info` 展示实际生效的 overlay；项目的本地 overlay 优先于继承的 Git overlay。
存在项目配置时，仅设置 `SHINE_CONFIG_DIR` 不会覆盖 `preset link` 选定的目录；
`SHINE_PRESETS` 仍具有更高优先级。`preset unlink` 移除保存的设置，但环境覆盖和继承设置
仍然适用，因此下次运行可能继续使用外部预设目录。
公共 `Preset Source` 与 `Presets Overlay` 提示会转义所显示路径中的控制字符。
`update --pull` 和 `upgrade --pull` 的 verbose 输出也会转义 Shine 的 Git 进度及跳过提示中的路径；
Git 自身输出的格式保持不变。
保存的路径及来源优先级保持原有含义。

发布预设前使用 `validate`、`lint`、`plan` 和 `test`。这些命令只检查创作输入，不会安装预设。
`migrate --dry-run` 用于预览旧 metadata 迁移，实际应用前需要审阅并确认。Git 管理的 overlay 是
可丢弃镜像，应在上游 checkout 中编辑，而不是修改镜像。
文本汇总按数量使用单复数，例如 `1 category` 和 `1 case`。
JSON 字段名和退出规则保持不变，包括 `lint --deny-warnings` 的警告退出策略。
`preset pack` 文本报告会转义目标名中的控制字符，将目标保留在同一行。
JSON 报告和包内容保留原始目标名。
如果 `preset schema` 无法写入输出，它会报告错误并以非零状态退出。
迁移输出的 artifact 后续操作命令会引用分类名：Unix 使用 POSIX shell 语法，Windows 使用
PowerShell 语法。迁移本身不会执行这些命令。
迁移汇总和 diff 预览会转义名称、路径及行内的控制字符。这些 diff 用于审阅，不能直接作为补丁；
JSON 中的身份信息和迁移写入的 metadata 保持原有含义。目标含控制字符时，迁移和 Plan 阻断后的
信任提示会要求使用原始参数，不提供可复制命令。Development trust 提示也会转义控制字符，
但不会改变信任授权或批准操作。
新模板不会生成空表或推测性能力清单。`--unrestricted` 会添加可选的 unrestricted 作者说明，
但无论是否使用，所有任意代码都未隔离。

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
shine env workspace export --format files [--workspace <FILE>] --mode <MODE> --file-group <NAME>... [--dry-run]
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
`--overlay` 只允许手动目录。默认写入会拒绝被高层遮蔽的键；显式目标允许维护被遮蔽项，
输出会分别报告保存位置和当前有效来源。`set`、`delete` 与 `secret encrypt` 的旧 `--force`
已停用并拒绝零写入；请改用明确的范围与文件参数。

`env list --reveal`、`env get`、`env secret decrypt`、带 `--include-secrets` 的导出，以及 `env run`
启动的子进程都可能接触明文。只在可信终端运行，也不要把密钥直接写入命令参数或文档。

缺失键和解密诊断会转义键名中的控制字符。`env get` 和 `env secret decrypt` 成功时仍输出原始值。
Broker 策略查找错误及本地 `env broker describe` 的来源行也会转义控制字符；来源摘要和策略匹配不变。

Broker policy 命令及完整远程流程见
[SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。本地变量、工作区加密、命令包装和 hybrid
后端见 [管理环境变量与密钥](../guides/environment.md)。

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
Shine 的任务启动及工作目录错误会转义名称和路径中的控制字符；保存的 argv、工作目录选择、
子进程输出和退出状态保持原有行为。

参见 [任务与应用构建产物](../guides/tasks-and-serve.md)和
[同步终端主题](../guides/terminal-theme-sync.md)。

## 清理

### 维护提醒

Shine 会在普通交互式调用时检查需要手动处理的维护事项。检查支持每次调用、每分钟、每小时、
每天四档频率，到期后在调用 Shine 时触发，无需常驻后台服务。

旧 HTTP 服务的注册文件会在每次符合条件的调用中检查；系统服务状态最多每小时由短生命周期
后台进程检查一次，结果在后续调用时显示。同一维护事项通常每天最多提醒一次。检查只读取状态，
不会移除服务、删除文件或执行预设。

若想立即运行全部现有维护检查，即使配置无法加载，也可以执行：

```text
shine state check
```

stderr 支持颜色时，维护提醒和检查未完成的警告会以加粗黄色显示，整组提示与命令输出之间
空一行。设置 `NO_COLOR` 可关闭颜色。

自动提醒写入终端 stderr。非交互调用，以及帮助、补全、预设创作、环境命令、命令运行器、
SSH/传输、主题同步、程序管理、迁移、清理和恢复命令会跳过自动提醒。
显式执行 `state check` 支持重定向输出，并会立即重复显示发现的事项；未能完成的检查也会在此
报告。自动检查失败不会阻断普通命令。

若 `state check` 检查期间在同一配置目录又执行了另一轮检查或清理，较早的检查会提示重试并
返回非零状态，不会报告检查已完成。

现有版本检查和补丁版本强制升级策略保持不变。维护检查也不会替代操作前的权限、信任、
兼容性或审批检查。

### 移除旧 HTTP 服务

```text
shine clear serve
```

`clear` 必须指定目标；单独运行不会清理所有内容。`clear serve` 会停止并移除旧版用户服务，
无需加载 Shine 配置，也不会删除已发布的文件。成功后重复清理也是安全的；系统服务管理器
报错时会报告失败。整个 `shine serve` 命令已移除。参见
[安装与升级](../installation.md#清理旧版本的本地-http-服务)。

成功清理后，会重置所选配置目录中的相关提醒缓存，并丢弃尚未完成的旧检查结果。
检查该状态时，请使用相同的 `--config-dir` 或 `SHINE_CONFIG_DIR`。

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

SSH 连接错误和会话清理提示会转义诊断文本及路径中的终端控制字符。
传输输出仍保留外部工具的原始格式和退出状态。

参见 [SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。

## 安装和升级 Shine

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

`stable` 跟随正式发布，`preview` 跟随会被持续替换的预览构建。参见
[安装与升级](../installation.md)。

文件组的配置、封存、持久导出、临时执行和清理边界见[临时机密文件](../guides/environment.md#workspace-secret-files)。
