# Bun Shell Preset PRD

## 1. 背景

`shine shell` 当前只接受 `.sh` 与 `.ps1` 作为预设入口。它们适合直接调用或 source 到当前 shell，
但对跨 Windows、macOS 与 Linux 的轻量命令编排并不友好：同一能力往往要维护两份实现。

Bun 能直接运行 JavaScript/TypeScript，并提供跨平台的文件、网络、进程和 shell API。此功能让
shell preset 在不引入包管理、编译或自动下载的前提下，以 Bun 脚本提供统一命令入口。

## 2. 产品目标

- 一期支持以 Bun 运行 `.ts`、`.js`、`.mts`、`.mjs` shell preset 入口文件。
- 用户始终通过无扩展名的稳定命令调用，例如 `mytool [ARGS...]`，而非手写 `bun <path>`。
- 同一预设在 Windows、macOS 与 Linux 使用同一份 Bun 源码、同一命令名与相同参数语义。
- 保持现有 shell preset 的提取、overlay、模板渲染、冲突保护、升级和安全卸载行为。
- Bun 是显式外部前置条件；shine 不安装 Bun、不联网下载运行时或依赖、不修改 package manager 状态。

## 3. 非目标

- 不支持任意语言或通用 `runner` 字段；一期只支持 `runtime = "bun"`。
- 不自动安装或解析 `node_modules`、npm 包、远程 URL import 或 lockfile。
- 不支持让 Bun 脚本与 `needs_source = true` 同时使用。
- 不把复杂、长期运行或高频低延迟能力塞入 shell helper；它们仍应使用独立 CLI、app preset 或服务。
- 不改变已有 `.sh` / `.ps1` 的安装方式和兼容性。

## 4. 预设接口

在 `shell/<category>/shine.toml` 的 `[[files]]` 中新增可选字段 `runtime`：

```toml
description = "Cross-platform Bun helpers."

[[files]]
source = "my_tool.ts"
target = "mytool"
runtime = "bun"
platforms = ["unix", "windows"]
```

规则：

- 缺省 `runtime` 时，维持当前行为：`source` 必须是 `.sh` 或 `.ps1`。
- `runtime = "bun"` 时，`source` 必须为 `.ts`、`.js`、`.mts` 或 `.mjs`；路径仍必须相对分类目录，
  且不得包含 `..`。
- `target` 沿用现有可选语义；未提供时使用源文件 stem。它必须是一个普通文件名，不能包含路径分隔符。
- `needs_source = true` 与 `runtime = "bun"` 是无效组合，加载预设时必须报出明确错误。
- `platforms` 沿用现有匹配规则。Bun 预设可声明两端，也可仅针对特定平台。
- 脚本可与同分类目录下的其他模块、数据文件一起嵌入/提取；只有 `[[files]]` 中列出的入口生成命令。

### 4.1 `needs_source` 边界

`needs_source` 的语义是修改**当前**交互 shell 的环境。Bun 作为子进程不能修改父进程环境，
所以 Bun 入口不能声明此能力。需要设置代理、PATH 或其他 session 环境变量的预设，继续使用最薄的
`.sh` / `.ps1` wrapper；该 wrapper 可调用 Bun 计算值或生成 shell 代码，但负责应用结果。

## 5. 用户体验与运行时行为

安装上述预设后，用户调用：

```text
mytool arg1 arg2
```

shine 在 `~/.shine/bin/` 安装受 shine 管理的启动器，将参数原样传给实际脚本：

```text
bun <effective-script-path> arg1 arg2
```

`effective-script-path` 是模板渲染后的副本（若脚本声明启用模板），否则是预设源文件。
因此现有 embedded preset、外部 preset 和 overlay 的有效来源规则不变。

**模板 opt-in 约定（重要，不能沿用现状不变）**：现有 shell 模板由 `# shine-template: true` 注解
经 `parse_template_annotation` 解析，它要求一行恰好是 `# shine-template: true`，且在首个非 `#`
注解行处停止扫描。JS/TS 的注释是 `//`，`# ...` 在 `.ts`/`.js` 顶层是语法错误，因此 Bun 入口
**无法**沿用该注解开启模板。

一期采用 **metadata 驱动** 方案（架构统一优先，不追求最小改动）：在 `shine.toml` 的
`[[files]]` 上新增 `transforms = ["template"]` 字段，与 app preset 完全一致地经
`install_core::apply_transforms` 应用，彻底不依赖脚本内注释。理由：Bun 入口本就必须在
`[[files]]` 中显式声明 `runtime = "bun"`，模板声明放在同一处最自然；且这把 shell 与 app 两条
预设线的模板 opt-in 收敛到同一 `transforms` 契约，消除"注解 vs metadata"两套机制的长期维护
分叉。`@@VAR@@` 替换、未定义变量报错终止安装等语义与现有 `template` transform 完全一致。

现状 shell 预设的 `FileToml`/`ShellFile` 尚无 `transforms` 字段（该字段今天只存在于 app 预设），
因此需要为 shell metadata 新增该字段并接入渲染路径——见 §7。既有 `.sh`/`.ps1` 的
`# shine-template: true` 注解路径保持不变，向后兼容；`transforms` 是面向新入口（含 Bun）的统一
声明式入口，两者可共存。

- Unix：生成无扩展名、可执行的 `mytool` wrapper；它使用 `exec bun ... "$@"`，保留退出码和信号语义。
- Windows：生成 `mytool.ps1` 和 `mytool.cmd`，分别将 PowerShell 的 `@args` 或 cmd 的 `%*` 原样转交给
  `bun`；用户通过 PATH/PATHEXT 直接运行 `mytool`。
- 启动器在执行 Bun 前检查 `bun` 是否可在 PATH 中找到。缺失时输出统一、可操作的错误（说明命令与
  脚本路径，提示安装 Bun），并以 `127` 退出；不得尝试安装或下载。
- 一期只检查运行时存在，不在每次执行时解析并强制 Bun 版本。预设作者必须在其说明中写明所需 Bun
  基线；未来需要版本门槛时再增加独立的 metadata 契约。
- `shine shell list` 对 Bun 入口显示 `runtime: bun`，并显示当前平台上 Bun 是否可解析；运行时
  不可用不阻止查看或安装预设。注意：当前没有 `shine shell info <category>` 子命令
  （`ShellCommands` 只有 `Init/List/Install/Reinstall/Uninstall`），已安装项的详情走顶层
  `shine info <TARGET>`（`show/` 模块）。一期不新增 `shell info`；如需在 `list` 中展示运行时
  状态，需要在 `shells/report.rs::handle_list` 增加渲染并对 `bun` 做一次 PATH 探测。

## 6. 安全、冲突与生命周期

现有 Unix 入口是 symlink，Windows 入口是已标记的 shim。Bun 启动器是普通文件，必须显式区分它与
用户文件，不能因为安装或卸载而误删用户内容。

**关键前置事实（一期核心工作量，不能当作沿用现状）**：Unix 侧当前**只会创建 symlink**，
`bin_links` 在 Unix 上没有任何"受管理普通文件"的生成/识别路径——内容型 shim 只存在于 Windows
（`create_windows_shims`）。因此：

- `bin_links::unlink_managed` 在 Unix 上把**所有非 symlink 一律 skip**（"用户文件永不删除"），
  归属判断仅基于 `read_link` 目标是否 `starts_with(managed_root)`。若不新增普通文件分支，Bun
  启动器（`exec bun … "$@"` 普通文件）会被判为用户文件而**永不被卸载**，`shell uninstall` 后
  留下孤儿命令。
- 当前性判断在 Unix 上也基于 symlink 目标相等（`link_executables_with_names`）。普通启动器需要
  按生成内容做**字节比对**，否则每次 `shine upgrade` 都会重写并误报变更。

一期必须新增一条 Unix 受管理普通文件路径，镜像 Windows 侧的
`windows_shim_status`/`shim_target`/`unlink_managed`：复用同一 `# shine-managed` 标记与
`# shine-target: ` 记录；普通文件"受管理"当且仅当带标记且其记录来源 lexically 位于
`managed_root` 之下；"当前"当且仅当字节等于按当前格式重新生成的内容。这是本功能最大、风险最高
的实现面（做错要么留孤儿、要么删用户文件，直接触碰 `docs/kb/architecture/invariants.md`
"Uninstall never touches user files" 不变量）。`shells/uninstall.rs` 现有对 `unlink_managed` 的
两次调用（managed-presets-root + rendered-root）也须接入这条新的普通文件分支，而不仅是 symlink
分支。

- 启动器包含稳定的 `shine-managed` 标记、运行时类型和有效脚本路径；Windows 的 `.ps1` 与 `.cmd`
  使用同一来源信息。
- 安装仅在入口不存在、或现有入口可验证为同一 Bun 预设的当前/过期启动器时更新。其他普通文件、
  目录、symlink 或指向不同来源的 managed 启动器均视为冲突；仅 `--force` 可覆盖。
- 现有 link 冲突展示继续指出占用的命令名、期望来源和 `shine install shell/<category> --replace-managed` 修复命令。
- 升级会在脚本有效来源、模板渲染结果或启动器格式变化时刷新启动器；同一目标的旧启动器需被识别为
  stale，而不是误报用户冲突。
- 卸载仅移除能验证为 shine 管理、且其记录来源位于当前 managed preset/rendered root 的 Bun 启动器；
  被用户编辑、缺少标记或指向外部位置的文件一律保留并报告 skipped。Windows 成对清理 `.ps1`/`.cmd`。
- 既有 `.sh`/`.ps1` symlink 与 shim 的识别和移除语义保持不变；新识别规则不得放宽对用户文件的保护。

## 7. 实现方案

1. 扩展 shell metadata：为 `ShellFile` 和 TOML 文件项携带可选 runtime，集中校验扩展名、`needs_source`
   互斥规则和平台筛选；无 metadata 的自动收集仍只收集现有 `.sh`/`.ps1`，避免意外把辅助 `.ts` 变成命令。
   扩展名门禁分布在**两个独立位置**，都要改：`metadata.rs` 的 `is_shell_script` +
   `normalize_shell_source`（后者对显式 `source =` 硬失败 `must end with .sh or .ps1`），以及
   `bin_links.rs` 的 `LINKABLE_SCRIPT_EXTENSIONS`/`EXECUTABLE_EXTENSIONS`。此外 `bin_links::link_stem`
   只对已知可链接扩展名剥离后缀，所以未提供 `target` 时 `foo.ts` 的默认命令名会保留 `.ts`——默认命令名
   的 stem 推导是第二个易漏点。同时为 `FileToml`/`ShellFile` 新增可选 `transforms: Vec<String>` 字段
   （现状 shell metadata 无此字段），作为模板 opt-in 的统一声明式入口（§5）。
2. 扩展链接规格，区分直接链接与 Bun launcher。shell 安装在解析有效源码/渲染文件后，为 Bun 入口创建
   launcher，而既有脚本继续走当前链接代码路径。模板渲染改为按 metadata 驱动：当 `[[files]]` 声明
   `transforms = ["template"]` 时，经 `install_core::apply_transforms(&["template"], ...)` 渲染到
   `rendered_dir`（与 app preset 同一入口），launcher 指向渲染副本；既有 `.sh`/`.ps1` 的
   `# shine-template: true` 注解路径保持并存、不回归。`build_link_specs` 的 `rendered.exists()` 有效
   来源选择规则不变。
3. 在 `bin_links` 中实现 Bun launcher 的创建、当前性比较、`--force` 覆盖和受标记卸载；以来源路径和
   runtime 共同验证归属。Unix 和 Windows 分别生成符合本平台参数转发规则的 wrapper。
4. 将 Bun 入口纳入 shell 的 upgrade、list、冲突提示及 uninstall 路径（无 `shell info` 子命令），
   也不修改 managed shell profile。Bun 入口不进入 `installed_source_commands`——注意该过滤器
   （`installed_source_commands_for_categories`）按 `file.needs_source` 而非 runtime 过滤，因此这条
   保证是**经由加载期 `needs_source + bun` 互斥校验传递而来**，而不是直接的 runtime 判断：只要
   §7.1 的加载期拒绝到位，Bun 入口恒为 `needs_source = false`，自然落出该过滤器。可选加一道
   `runtime != bun` 的直接护栏作为双保险。
5. 更新 `shine preset new shell` 模板、preset authoring 文档与 README，提供零依赖 TypeScript 示例，并明确：
   不使用外部包、不得依赖自动安装、环境变更必须保留 shell wrapper。

## 8. 验收与测试

- Metadata 单测：接受四种 Bun 扩展名；拒绝未知 runtime、非 Bun 扩展名、路径穿越及
  `runtime = "bun" + needs_source = true`；解析 `transforms = ["template"]` 字段；回归验证
  `.sh`/`.ps1` 及既有 `# shine-template: true` 注解路径不变。
- Unix 链接测试：安装生成可执行无扩展名 launcher，参数（空格、引号、通配符与 dash-leading 参数）原样
  转发；不存在 Bun 时返回 127 且不执行脚本；正确来源的 launcher 幂等、stale 可刷新、用户文件需
  `--force` 才覆盖。
- Windows 单测：验证 `.ps1` 与 `.cmd` 内容调用 Bun、保留 `@args`/`%*`，并正确识别、更新和成对卸载。
- 生命周期测试：embedded、外部 preset 与 overlay 都能生成 Bun 入口；声明 `transforms = ["template"]`
  时 wrapper 指向 rendered 副本（`@@VAR@@` 已替换、未定义变量报错终止安装）；`shell uninstall --dry-run`
  不写入；实际卸载不删除用户改写的 wrapper。
- CLI 验收：`shell list` 显示 runtime 状态（无 `shell info` 子命令）；`shell install`、`upgrade`、
  `reinstall` 与 `uninstall` 的输出保持现有格式和冲突保护。

## 9. 默认决策

- 一期唯一新增运行时为 Bun；不为 Node、Deno、Python 或通用 runner 预留用户可见接口。
- Bun 脚本应优先用 Bun 标准 API 和 Bun Shell 实现跨平台逻辑，避免依赖 Unix 专属外部命令。
- Bun 缺失是可安装但不可执行的状态：可发现、可诊断，且绝不触发隐式环境变更。
- 预设的复杂业务边界维持在 shell helper 之外；本功能只改善轻量命令编排的实现语言与跨平台交付。
