# Live Bun 单次内部启动 PRD

> **状态：代码已实现，macOS、Linux 全量测试、三平台真实 Bun 终端交互及 Windows 原生启动/迁移检查已通过；Windows 全量测试与 Clippy 的既有失败已记录。** 日期：2026-10-09。
> 本文记录 Live Bun 启动链的改造目标；命令、字段及迁移方式已落实到代码，尚未发布。实施证据见 [验证报告](live-bun-single-launch-validation.md) 和 [ADR 0102](kb/decisions/0102-single-process-live-bun-launch.md)。
> 本文属于内部设计资料，不纳入公开网站，也不授权执行、安装或信任外部预设。

## 1. 背景与问题

Shell 预设的运行时与部署方式是两个独立选择：`runtime = "bun"` 表示通过 Bun 执行
JavaScript 或 TypeScript，`external_shell_mode = "live"` 表示源代码修改在下次调用时生效。
Live 命令若声明 transforms，需要先渲染最新源文件；若同时声明非空 env，还需要解析并注入
这些变量。当前启动器组合两个已有 CLI 入口完成这两项工作：

```bash
shine --config-dir '/data/shine-a' __shell-render 'shell/demo/run'
exec shine env run --no-workspace --with 'API_TOKEN' \
  -- bun --no-install '/data/shine-a/rendered/shell/demo/run.ts' "$@"
```

上例采用自定义状态目录；现有模板还会在推导出的目录名称为 `.shine` 时省略渲染调用的
`--config-dir`，即使该目录并非默认目录。变量注入调用则始终未传入同一目录。
第一个 Shine 进程中的环境设置不能回传给父启动器，因此第二次调用可能读取调用者指定的
另一状态目录或默认状态目录。
结果可能是缺失变量而启动失败，也可能是静默使用另一套同名配置或凭据。

两次调用还重复承担进程启动、参数解析、Tokio runtime 建立及配置发现和加载成本。
现有渲染入口通过通用 Core runtime 捕获外部预设树及 overlay；这部分成本不会仅因合并
CLI 调用而消失。性能优化应分别测量重复初始化与预设捕获，不能预先承诺耗时降幅。

代码依据：

- [启动器生成](../core/src/runtime/launcher.rs)：`unix_live_launcher_content`、Windows shim helpers。
- [渲染适配器](../cli/src/shells/deployment.rs)：`handle_render_live`。
- [受安装记录约束的渲染](../core/src/runtime/shell.rs)：`render_live_shell`。
- [环境解析与子进程执行](../cli/src/env/workspace.rs)：`resolve_explicit_values`、`command_status`。
- [配置加载与命令路由](../cli/src/main.rs)。
- [配置分层及初始化](../cli/src/config/load.rs)、[配置内存视图](../cli/src/env/mod.rs)：
  `EnvConfig::load_or_init(config)` 当前仅复制 `config.env`，不会再次读取磁盘配置。
- [安装记录和 schema 门禁](../core/src/runtime/shell.rs)：`ShellManifestEntry`、
  `load_shell_manifest_with_host`；[Action receipt 与 journal](../core/src/runtime/shell_action_executor.rs)。

## 2. 目标与范围

目标是让需要即时渲染的 Live Bun 命令在一个 Shine 进程内完成：一次配置加载、安装记录校验、
受约束渲染、声明变量解析及 Bun 启动。渲染与变量解析使用同一份已加载配置，安装状态目录
明确绑定，当前工作目录对应的项目覆盖继续生效。

首轮覆盖安装记录为 Live、Bun、非空有效 transforms 且 `needs_source = false` 的 Shell 命令，
包括 env 为空的情况。以记录中生效的 transforms 为准，包含安装时由模板标记推导出的管线，
不能只检查源 metadata 是否显式声明 transforms。
其中非空 env 的启动链从两次 Shine 调用改为一次；env 为空的命令统一进入新启动入口，
并验证新增记录解析开销。支持 macOS、Linux 和 Windows 的对应启动器。

首轮不扩展到 Native/source 命令、Snapshot 启动器或无 transforms 的直接启动器。
它们继续保持现有执行方式。需要覆盖这些路径的配置绑定问题时，另行设计其迁移范围。
因此本轮只关闭已迁移的上述路径的配置绑定问题；旧格式及其他路径的同类问题仍须明确记录。

非目标：

- 不增加公开的通用命令执行接口，不改变预设元数据语法。
- 不修改安装、升级、卸载及恢复的审批、所有权与信任边界。
- 不新增凭据缓存、后台常驻进程或整套安装事务。
- 不将任意 Bun 代码描述为沙箱执行，不承诺捕获全部外部依赖或运行期间的源文件变化。
- 不在本轮实现按命令缩小预设树捕获或每次调用的独立执行快照。

## 3. 内部命令契约

候选隐藏命令为 `__shell-launch`，生成的 Unix 启动器形态为：

```bash
exec shine --config-dir '/data/shine-a' \
  __shell-launch 'shell/demo/run' -- "$@"
```

入口仅接受完整 canonical target `shell/<category>/<command>`，拒绝类别缩写、空组件、额外组件、
`.`、`..`、反斜杠及控制字符，不做路径归一化后再猜测目标。要求 `--` 作为脚本参数边界，
允许其后零个参数；其后的 `--config-dir` 等内容不能再作为 Shine 全局选项解释。
参数进入 Rust 后保留为 `OsString`，直接传给子进程，不拼接成 Shell 命令或重新解释。
Unix 验证非 UTF-8 参数；Windows 分别验证 PowerShell 参数数组与 CMD 的 `%*` 转发，
只承诺各 Shell 能表示的参数通过启动器边界后不再被 Rust 层损坏，不承诺还原 Shell 已消耗的语法。

入口不接受任意脚本路径、runtime、transforms、env specs、Bun dependency mode 或任意
子进程 program。runtime、路径、transforms、env specs 和依赖模式全部从已安装记录取得；
runtime 必须映射到实现内固定的 Bun 启动方式，不能将记录字符串直接当作 program 或 Bun 选项。
用户参数只作为该脚本的参数传递。
隐藏命令仍须校验输入，隐藏属性不是权限保护。

启动器始终显式传入安装时确定的绝对状态目录。不得仅因目录名为 `.shine` 就省略绑定。
新格式的重建须能从明确的安装上下文及记录取得此目录，不从任意命名路径反推。
实际加载的 `shine_dir` 必须与该绑定一致；项目配置可以覆盖现有可覆盖项，但不能将安装
manifest、锁、journal 或 rendered 根切到项目的另一目录。本轮沿用通过 PATH 查找 Shine
的方式；PATH 中旧版 Shine 不支持新入口时应失败，不回退执行脚本或自动改写启动器。

## 4. 执行流程与失败语义

```text
解析内部命令及原始用户参数
    ↓
加载一次全局配置和当前项目覆盖
    ↓
取得现有跨进程操作锁
    ↓
检查 Shell journal，读取并校验目标安装记录
    ↓
校验执行字段、路径及 env 声明，再按记录执行受约束的 Live 渲染
    ↓
捕获执行描述并释放操作锁
    ↓
使用同一配置解析记录声明的变量，必要时解密
    ↓
直接启动 Bun，继承终端并传递退出状态
```

必须满足以下约束：

1. 配置只进行一次加载流程，保留现有全局、项目及 env override 的优先级。具体为绑定目录的
   `config.toml` → 当前工作目录发现的 `shine.config.toml` → 全局、overlay、项目
   `shine.env.toml` 覆盖。渲染和 env 解析不再次调用 Config 加载器；从 Config 构造 EnvConfig
   允许复制内存，不算再次加载。一次流程不意味着当前加载器对每个文件只读一次，也不意味着
   多个配置文件被原子捕获。不自动发现 `shine.workspace.toml`，不触发后台版本网络查询。
   本轮沿用 `Config::load_or_init` 已有目录初始化及默认值补齐行为，入口不能被描述为只读；
   不增加安装、升级、信任登记或配置迁移副作用。若另行改为无写入加载，须单独定义并测试
   默认值和分层等价性，不能直接替换成省略项目及 env override 的全局 dry-run 加载器。
2. 在现有操作锁内检查 pending Shell journal、读取记录并进行渲染。缺失记录、非 Live、
   非 Bun、`needs_source = true`、未知格式或依赖模式、无效 transforms/env specs 及其他不符合
   范围的目标必须拒绝。重复 canonical target 的记录不能以“第一个匹配项”执行。
   校验在渲染写入及解密前完成，不降级为任意路径执行。
3. 渲染仅使用记录指定的源文件及 transforms，输出须满足现有受管 rendered 路径约束。
   源路径及 rendered 路径须为绝对路径；不能只用字符串前缀证明归属，必须拒绝 `..` 越界、
   非普通文件及违反现有链接策略的目标。内容相同不重写；读取或 transform 失败时保留最后
   有效文件并停止本次执行。原子写入、权限设置或持久化同步报错也停止执行，但替换后报错
   时新字节可能已可见，不能声称全部 I/O 失败都回滚为旧文件，不运行旧内容作为失败回退。
4. env 与依赖策略来自这次捕获的安装记录。源元数据变化不能在调用时隐式扩大变量注入
   范围或改变依赖模式；部署元数据变化继续通过安装或升级应用。
   Bun 选项仅由记录模式映射为 `--no-install` 或 `--install=fallback`。按 ADR 0031，Live
   包及锁文件仍可随源变化，`dependency_hash` 不是调用时的字节冻结或拒绝变化门禁；本轮
   不搬运依赖文件、不改变模块解析目录，不运行 `bun install`，不取得 Bun 缓存所有权。
5. env 为空时不解析声明或解密，但 template 渲染仍读取同一 Config 的原始 env 表。
   非空时复用现有显式变量解析、加密值优先、`SOURCE=TARGET` 别名及失败即停止的语义；
   无效名称、重复目标或缺失存储值拒绝执行，继承的同名进程变量不能充当缺失值回退。
   显式注入覆盖继承的目标变量，其他进程变量继续继承，不将整个配置 env 表注入 Bun。
   template 不自动解密，也不受命令 env 声明限制；解密结果只用于子进程注入，不回填 Config
   或渲染输出。变量值、密文和解密明文不写入安装记录、journal 或诊断。
6. 解密交互和 Bun 运行期间不持有操作锁。Bun 启动失败明确报错；继承 stdin/stdout/stderr，
   不改变调用者 cwd。缺少 Bun 保留现有 `127`，其他准备或 spawn 错误为非零；正常结束
   保留 Bun 的退出码。Unix 子进程被信号终止时按现有 env runner 映射为 `128 + signal`，
   这不等同于 Shine 自身由该信号终止。Windows 保留平台相应的退出语义。
   Ctrl-C、终端输入及 Shine 自身被终止须有真实进程测试；不能仅凭继承 stdio 宣称信号与
   子进程生命周期完全正确。env 为空的旧路径会直接 exec Bun，新路径增加等待的 Shine
   进程，需单独验证该变化；等待期间尽早释放不再使用的预设捕获与渲染缓冲。
7. Live 部署继续遵循 Development trust 要求。内部入口不增加信任例外，不替代安装审批，
   也不因合并调用额外添加用户确认流程。
   当前 `render_live_shell` 本身没有调用时重新审批或重新检查 Development grant 的流程，
   本轮不得把既有安装/升级信任门禁描述成每次启动都会重新校验，也不得借新入口申请信任。

渲染与 env 注入并非联合事务：渲染成功后，缺失变量、解密失败或 Bun 启动失败不会撤销
已提交的 rendered 文件；失败只保证本次不启动 Bun。安装记录仅捕获一次，解密后不得
重新读取另一份记录来改变路径、声明或依赖选项。

配置一致性只保证这次调用使用同一份加载结果。释放锁后，其他调用仍可能更新共享渲染文件，
外部 Live 源及依赖也可变化。实现须评估并记录这项现有并发边界；若要保证 Bun 消费本次
捕获的确切字节，需要独立执行快照设计，不能通过长期持锁或本轮性能改造隐式引入。

## 5. 模块职责与实现方向

| 模块 | 改造职责 |
| --- | --- |
| `core/src/runtime/shell_launch.rs`、`shell.rs` | 提供安装记录约束的 Live Bun 执行准备接口；锁内只捕获一次 receipt，复用 journal 检查和渲染逻辑，返回受支持的执行描述 |
| `core/src/runtime/launcher.rs` | 为范围内命令生成三平台新格式启动器，显式传入安装状态目录；生成、inspection、planner 和 executor 使用同一格式判定，保留旧格式确定性重建 |
| `cli/src/commands/cli.rs`、`cli/src/main.rs` | 增加隐藏入口及一次配置加载的路由，跳过常规版本通知 |
| `cli/src/shells/deployment.rs` | 将同一配置用于 Core 渲染准备、变量解析及 Bun 执行 |
| `cli/src/env/workspace.rs` 或独立内部 helper | 抽取可复用的显式变量解析和进程执行函数；env 为空直接走进程 helper，不调用要求非空 `--with` 的 workspace handler，不派生 `shine env run` |
| Shell planner、action executor 及 receipt 模型 | 将格式迁移纳入原有 Plan、精确资源匹配、更新事务和恢复 |

Core 不依赖 CLI 解密后端或 Clap。执行描述是进程内的类型化数据，不是新增的序列化审批
能力，也不能把可执行路径或明文变量塞进安全 Plan。构建通用 runtime 的现有成本先保留，
后续按测量结果决定是否缩小加载范围。
执行描述至少包含校验后的 rendered 路径、Bun 依赖枚举及有序 env specs；不返回任意 program、
原始待执行命令行或锁 guard。准备接口自己完成渲染，不要求 CLI 在准备之后再次调用
`render_live_shell`，否则会重复加锁和读取记录。

## 6. 旧启动器与记录迁移

当前更新事务要求真实旧启动器与旧安装记录确定性重建出的资源精确匹配。
只替换生成模板会破坏这项证明，因此格式兼容必须先于启动器切换完成。

实现采用命令记录字段 `launcher_format = "live-bun-v2"` 与绝对 `launcher_config_dir`，
由 ADR 0102 固定。字段均缺省表示旧格式；仅范围内的新安装和经审批的升级写入新格式，
其余命令保留原格式。
旧、新格式均保留独立确定性重建，未知格式及与 runtime/mode/transforms 不相容的组合拒绝
处理。旧重建保留原模板的 `.shine` 省略规则和未绑定的 env runner 字节，不能为了修复问题
同时修改“旧格式”，否则现有真实资源将无法通过精确匹配。

现有 `ShellManifest` / `ShellManifestEntry` 不拒绝未知字段，而读写入口会拒绝未来 manifest
版本。因此采用新增执行格式字段时，**新格式必须由提升后的 Shell manifest schema 承载**，
不能仅在 schema v1 加一个可忽略字段。新实现读取旧 manifest 只在内存补齐旧格式，读取、
状态检查和内部启动均不因此重写 manifest；首次受审批的 lifecycle 写入才保存新 schema。
未选中的旧记录保持旧格式，不通过容器 schema 升级悄悄改写所有启动器。

格式标识须进入与执行有关的 receipt、Plan 指纹、Action IR 记录转换及恢复匹配。
升级必须能将仅格式变化识别为更新，通过现有 launcher 事务替换；修改过或外来的资源
继续作为冲突保留。Windows 两个 shim 均参与精确比对、审批和恢复。
新格式重建所需状态目录由安装、规划和恢复的显式上下文一致传入；如实现还需要新增 receipt
字段，它们也必须进入完整记录比对。不能让 status 仅比较源、env 等旧字段而漏掉格式更新，
也不能让 uninstall 或 recovery 用当前默认模板重建旧资源。

保留旧 `__shell-render` 入口，支持尚未升级的启动器。新启动器仅由支持新入口的版本生成。
Action receipt 及 Shell journal 当前采用 `deny_unknown_fields`；是否提升它们各自的 schema
须在 ADR 中按读取、校验和恢复路径决定，并用旧 reader 对新数据的拒绝测试证明门禁。
新 manifest 门禁不能代替 journal 独立兼容验证，因为中断可能发生在新 manifest 提交之前。
旧版本遇到新状态必须在修改前失败；保留状态及恢复材料，不自动降级格式。
对已有 pending journal，先按受支持的原格式显式恢复，再进行迁移，不在普通升级中隐式改写。
新实现也必须能恢复旧版 journal；无法支持的历史版本应明确失败并保留材料。

## 7. 验收与测试

| 场景 | 验收结果 |
| --- | --- |
| 安装目录 A，调用者 `SHINE_CONFIG_DIR` 为 B | 渲染及 env 注入均使用 A 的全局基座；项目覆盖仍按工作目录生效 |
| A/B 中存在不同同名变量，cwd 项目和 env override 再覆盖该变量 | 渲染值和注入值均符合 A 基座上的相同分层；项目不能重定向安装记录及输出根 |
| 自定义状态目录也名为 `.shine` | 三平台启动器始终显式绑定绝对目录，不用名称推断默认目录 |
| 平台合法路径含空格、单引号、`$`、反引号；Windows 另含 `%`、`!`、`&` | 按各自 Shell 语法引用，无变量展开或命令注入；Windows 不要求非法的双引号路径 |
| Live 源内容修改，元数据不变 | 下次调用使用更新的渲染内容，无须升级 |
| 源 metadata 中 env、runtime、transforms 或依赖策略修改 | 本次仍使用安装记录，不能隐式扩大注入、改变渲染管线或切换依赖策略 |
| 安装时由模板标记推导出 transforms | 即使 metadata 未显式声明，仍进入本轮新启动链 |
| Disabled / Locked 记录，Live 包与锁文件变化 | Bun 选项按记录映射；保持现有依赖文件及模块解析语义，不因 dependency hash 变化冻结 Live 内容 |
| 内容未变化，env 为空 | 不重写渲染文件，不调用解密；只有一次 Shine 启动 |
| env 为空但 template 使用当前配置变量 | 渲染正常替换，不将整个配置 env 表注入 Bun |
| env 别名、加密值与明文并存、继承变量冲突 | 加密值优先；只覆盖声明的目标，其他变量继承；继承值不能替代缺失存储值 |
| 缺失变量、解密失败、Bun 不可用 | 明确失败，Bun 不执行；缺少 Bun 为 `127`，已成功渲染的文件不回滚 |
| 读取或 transform 失败；替换、权限或同步失败 | 本次均停止；提交前失败保留旧文件，提交后错误不谎报回滚，无旧内容执行回退 |
| 缺失或重复记录、错误 runtime/mode/needs_source、非法 env/transforms、未知格式/依赖模式 | 渲染写入及解密前拒绝，不接受任意路径回退 |
| 非 canonical target、相对或越界路径、非普通文件、违反链接策略的输出 | 明确拒绝；不能以简单前缀匹配绕过受管输出边界 |
| pending、损坏或未来版本 Shell journal/manifest | 失败并保留状态，不渲染，不运行 Bun，按支持情况提示显式恢复 |
| 附近存在 workspace 定义 | 不自动发现或注入 workspace 变量 |
| 空参数、空字符串参数、前导 `-`、字面 `--`/`--config-dir`、Unicode、Unix 非 UTF-8 参数 | `--` 后不再解析 Shine 选项，参数在各 Shell 支持范围内保真 |
| 子进程 cwd、终端及退出状态 | cwd 不变、输入可用；测试成功、非零退出、Unix 信号映射、Ctrl-C 及 Shine 终止时的子进程行为 |
| 旧格式升级、更新中断、修改后的 launcher | 正常迁移；显式恢复有效；冲突文件保持原状 |
| Windows 原生 launcher pair | 两个资源均通过更新、卸载及恢复验证，实机 CI 验证执行语义 |
| 新 reader 读取旧记录/旧 journal，旧 reader 遇到新记录/新 journal | 旧资源按旧字节重建并可恢复；旧 reader 在修改前拒绝新状态，材料保持原状 |
| PATH 中 Shine 旧版或不存在；范围外启动器 | 旧版新入口报错或缺失 Shine 为 `127`，无执行回退；范围外模板及旧入口保持兼容 |

测试采用临时配置目录、受控源文件、合规信任 fixture 和本地 Bun 替身，避免激活开发者
真实预设或读取真实凭据。文本检查用于验证生成格式，真实子进程测试验证目录绑定、参数和
错误行为；Core in-memory host 测试验证记录约束、锁内准备及事务恢复。
用独立进程和同步屏障覆盖并发调用、调用与 upgrade/uninstall 的交错：准备持锁时 lifecycle
必须等待，pending journal 时准备必须失败；释放锁后的共享输出变化作为已声明边界记录，
不能误验收为“每次调用消费确切捕获字节”。通过观测加载入口和进程创建验证单次调用，不能
只统计模板文本中的 `shine` 字样。模拟解密失败并检查诊断、receipt 和 journal 不泄露测试值。

## 8. 性能验收

记录相同构建、机器、配置和受控预设树下旧、新启动链的启动耗时分布，分别覆盖小型和大型
预设树、渲染无变化和发生变化、env 为空和非空。不含硬件解密交互的基线与含凭据流程分开；
使用相同 Bun 替身排除业务执行耗时，标注冷热缓存、重复次数和构建模式。

结构性验收为每次成功调用仅启动一个 Shine 进程、仅进行一次 Config 加载流程，内部及生成
启动器不重新调用 Shine；解密 helper 或插件的必要子进程不属于 Shine 次数指标。
性能报告分别列出总启动延迟、配置加载、Core runtime/预设捕获和渲染准备成本，采用中位数
及尾部延迟比较。
不预设毫秒或百分比目标，但“单次启动”本身不能证明更快。测量前固定样本数、尾部分位和
回退判定方法，报告离散程度；若任一覆盖场景出现超出测量噪声的可重复回退，须完成原因
分析和缓解，或在性能报告中明确接受的取舍及依据，不能仅以调用次数达标关闭性能验收。
长期子进程的等待进程与驻留内存另行比较，尤其覆盖旧版直接 exec Bun 的 env 为空路径。

## 9. 实施顺序与完成条件

1. 确定格式标识、manifest 版本提升、Action/journal 兼容门禁和旧资源重建方式，形成实现 ADR；
   固定性能测量与回退判定方法。
2. 提取 Core 受记录约束的准备接口，复用既有渲染和 journal 边界。
3. 提取内部 env/进程 helper，实现单次配置加载的隐藏启动入口。
4. 切换 Unix、PowerShell、CMD 启动器并接入原有审批更新事务。
5. 完成行为、迁移、恢复和三平台检查，进行性能对比。
6. 在实现变更中更新内部 ADR、invariants、data flows、module map，并按实际发现补充 lessons。
   配置目录绑定修复属于用户可见行为，发布时须同步更新两种公开手册及手写 CHANGELOG；
   手册说明适用范围和正常 `upgrade` 行为，不公开内部命令、receipt/schema 操作或恢复内部细节。

完成条件：上述行为验收通过，旧启动器可安全升级及恢复，范围内已迁移启动器的配置绑定
问题关闭，单次启动结构性指标满足，性能回退已处理或有明确取舍依据。三平台对应执行及
迁移检查通过才可宣称完整实现；未验证平台必须标注为未完成，不能以模板检查替代实机执行。
实现阶段按改动范围运行仓库要求的检查；
仅保存此 PRD 不要求 Rust/Bun 测试或公开网站构建。

## 10. 相关设计

- [External Shell Snapshot 与 Live 模式](kb/decisions/0023-external-shell-snapshot-and-live-modes.md)
- [External Bun 依赖策略](kb/decisions/0031-locked-external-bun-preset-dependencies.md)
- [Shell 启动器创建事务](kb/decisions/0058-transactional-shell-launcher-creation.md)
- [启动器更新事务](kb/decisions/0059-transactional-shell-launcher-update.md)
- [启动器移除事务](kb/decisions/0060-transactional-shell-launcher-removal.md)
- [架构约束](kb/architecture/invariants.md)
- [Shell 部署数据流](kb/architecture/data-flows.md)
- [KB 维护协议](kb/README.md#how-to-update-this-kb-maintenance-protocol)
