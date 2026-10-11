# Live Bun 单次启动实施与验证

日期：2026-10-09。范围和失败语义以 [PRD](live-bun-single-launch-prd.md) 与
[ADR 0102](kb/decisions/0102-single-process-live-bun-launch.md) 为准。
代码、macOS/Linux 全量套件及 Windows 原生启动/迁移检查已完成。Windows 全量套件和
严格 Clippy 仍有未修改代码中的失败；三平台真实 Bun 终端交互检查通过，但硬件解密及
Windows 性能等边界仍未覆盖，不能宣称三平台完整验收。

## 已实施

- 隐藏 `__shell-launch` 入口保留原始 `OsString` 参数，要求目标后显式 `--`；一次分层 Config
  加载后，准备、显式变量解析和 Bun 执行共享该配置，不调用 workspace handler 或版本通知。
- Core 在既有操作锁内检查 pending journal、唯一安装记录、范围、格式、依赖模式、transforms、
  env 声明及路径；按捕获记录渲染。相同内容不重写，校验/读取/transform 失败不执行旧文件。
  输出替换后的持久化失败仍可能留下新文件，不承诺全部 I/O 失败回滚。
- CLI 在解密及等待前释放操作锁、Core runtime 和预设捕获；仅将声明的变量显式注入 Bun，
  复用加密值优先、别名、覆盖继承值及失败即停止语义。等待结束后清零显式值缓冲。
- 三平台新模板绑定安装目录，传递一次 canonical target；旧模板字节保持原样。
  CMD 新模板对配置字面量的百分号转义、禁用延迟展开，并跳过原始 ownership 注释，避免路径
  元字符参与执行。Windows 原生路径去除 verbatim 前缀；原生 CMD/PowerShell 检查已通过。
- Windows MSVC 二进制主线程栈预留 8 MiB，修复原生 debug CLI 启动/安装栈溢出；
  PE header 已核验。预留地址空间按需提交，不等于增加 8 MiB 常驻内存。
- Shell manifest 写入 schema 2，旧 schema 0/1 仅在内存归一化，未选择的旧记录保持 legacy。
  format/root 进入完整 receipt 比较和既有更新/恢复事务；Action/journal 保持 schema 1，旧的
  strict receipt 读取器拒绝新字段。没有运行时隐式迁移。
- Unix 保留调用者进程组及 stdio，向直接子进程转发父 PID 收到的 SIGINT/SIGTERM/SIGHUP，
  等待并回收子进程，以 `128 + signal` 映射退出。没有新增后代进程隔离或强制回收保证。

## 自动化证据

新增的 Core 与真实 CLI 测试使用临时配置、受控源码和本地编译的 Bun 替身，不读取真实凭据，
不执行开发者预设。覆盖：

- `.shine` 命名的非默认状态目录、另一个 `SHINE_CONFIG_DIR`、项目与 env override 分层，
  无 workspace 自动发现；同一命令只经过一次生成启动器的 Shine 调用。
- 空 env、别名覆盖、未声明变量不注入、空参数、Unicode、`--`、看似 CLI 选项的参数，
  Unix 非 UTF-8 argv；源元数据变化不改变安装记录的执行约束。
- 原始 template env、内容无变化不重写、内容变化重新渲染；无效执行字段、重复目标、未知
  transforms/env/依赖/格式、相对和越界路径、rendered 子树符号链接及 pending journal 拒绝。
- 缺失 env、损坏的加密值、无密文诊断泄漏、缺失 Bun 的 127、Bun 的 42 和信号退出码。
- 先等跨进程锁，再读 receipt；真实父 PID 的 SIGTERM、同一进程组的 SIGINT、继承 PTY 输入
  及直接子进程回收。该自动化套件的 PTY 输入和进程组 SIGINT 分开测试；真实终端驱动
  Ctrl-C 由后述真实 Bun 补测覆盖。
- schema-1 legacy 启动器的 approved upgrade、manifest 写入故障、pending 拒绝、显式 recovery
  恢复确切旧资源、重试迁移、无变化再次规划及卸载。此测试会随当前原生平台选择 Unix
  资源或 Windows 双 shim；已在 macOS、Linux、Windows 执行。
- 冻结的旧 Action/journal receipt 语法可以读取旧字段，且明确拒绝新 format/root 字段。

实际检查结果：

- 全 workspace/all-features Nextest：1439 项通过，2 项按配置跳过；其中 opt-in 性能测试另行运行通过。
- `cargo check --workspace`、`cargo fmt --check`、Clippy 全 targets/features/tests/benches 且 `-D warnings`：通过。
- `cargo deny check bans licenses sources`、`typos`、10 份 Markdown 的本地链接/空白检查：通过。
  依赖审计保留既有 unused license allowance 与重复版本警告，本次没有修改依赖。
- 网站 frozen-lockfile 安装、`pnpm check:locales`（各 21 页）、`pnpm typecheck`、英/中生产构建：通过。
  Docusaurus 的更新检查配置权限警告不影响构建。
- `git diff --check` 和最终工作树检查：通过；原有未跟踪资料保留，未提交或推送。

初次全量测试在沙箱内因既有 SSH 测试的 Unix socket bind 被拒绝；允许本地 socket
后完整套件通过。Cargo 的 sccache wrapper 在沙箱内也被拒绝，验证时使用 `RUSTC_WRAPPER=`。
这些环境限制没有通过修改产品代码绕过。

## Linux 原生验证

通过用户授权的 `tencentCloud` SSH 实例，在独立临时目录验证上传的工作树快照。
环境为 Ubuntu 24.04.4 LTS、Linux x86_64、内核 6.8.0-101-generic；4 vCPU、约 3.7 GiB RAM。
现有 Rust 1.96.0 与仓库固定版本完全一致（rustc `ac68faa20`、cargo `30a34c682`），
直接使用该工具链的 bin 目录，避免重复下载同版工具链。Bun 1.3.14 已安装；以下自动化
测试使用受控 Bun 替身，不执行实例已有 preset，也不读取真实凭据或激活现有配置。

源快照以 HEAD `562d1cb2f55e63e44eae8d5e146e51d6cac11c21` 加当前未提交改动生成，
不含本地配置、凭据或无关未跟踪文件。归档 SHA-256：
`ac6274c2e8fd67b10387a0064a85e1f3061c9d739c3120fb20a50d2d4d6c104c`。
归档中的 206 份 Rust/工具链/依赖锁文件与上传时工作树逐字节一致；Linux 与本地
`Cargo.lock` SHA-256 均为 `8aa22a8a239a85954fd3d9aa3b26ac9fba630ffe7e05172475fb8213e2e8b253`。
随后 Windows 验证增加 MSVC 条件编译的栈预留、PowerShell 模板参数修正及隔离测试目录修正；Linux 未重跑该后续
快照。栈预留不作用于 Linux；修正后的完整本地套件通过。

- 定向 `cargo test --locked --workspace --all-features live_bun`：15 项通过
  （9 项真实 CLI 进程测试、6 项 Core/receipt/模板测试）。
- 全量 `cargo test --locked --workspace --all-features -- --test-threads=2`：1436 项
  单元/集成测试通过，6 项文档测试通过，2 项按配置忽略。
- `cargo fmt --check` 和 Clippy 全 targets/features/tests/benches、`-D warnings`：通过。
- 包括 legacy schema-1 启动器批准升级、manifest 写入故障、显式恢复确切旧资源、重试迁移
  和卸载；还包括锁等待后捕获记录、配置目录绑定、参数/别名、继承 PTY 输入、SIGINT/
  SIGTERM 退出及直接子进程回收。没有用 Windows 模板测试替代 Windows 执行。

首次 SSH 连接在编译时超时，后续采用连接保活并将结果写入临时目录日志。
第一次后台全量执行有 1 项既有 Core 终端测试超时；原因是后台 shell/`nohup` 继承了忽略
SIGINT 的处置。相同二进制在前台单独复测通过，后台命令加 GNU
`env --default-signal=INT,QUIT` 后完整复测通过。没有修改测试或产品代码来隐藏该问题。
后台复现测试时也应恢复默认信号，否则真实 Ctrl-C 测试无法使用正常终端语义。

## macOS 性能方法

同一台 macOS arm64 机器、同一个本地 debug 二进制，对比冻结的 legacy 模板与新模板。
使用隔离状态目录和相同的无业务代码 Bun 替身，样本计时不包括修改源文件；无硬件解密。
小树只有目标源码，大树额外含 500 个 4 KiB 文件；配置、source 字节长度和 dependency mode
一致。每个场景先热身 5 对，再记录 40 对，旧/新运行顺序交替。没有清除 OS 文件缓存，
因此这是热缓存基线，不包含真正冷缓存或发布构建数据。

测量前固定统计：中位数、nearest-rank P95、MAD；可重复回退阈值为
`max(旧中位数的 5%, 3 × 较大的 MAD)`。额外在同一进程内测 Config 加载、通用 Core
runtime/预设捕获和准备的阶段中位数，这些不是子进程总耗时的精确拆账。
以下为最后一次完整矩阵的结果，单位 ms：

| 额外文件 | env | 渲染变化 | 旧中位数 / P95 / MAD | 新中位数 / P95 / MAD | Config | Core 捕获 | 准备 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 无 | 无 | 21.125 / 22.213 / 0.467 | 20.498 / 22.286 / 0.509 | 0.828 | 0.539 | 0.684 |
| 0 | 无 | 有 | 31.914 / 32.849 / 0.581 | 30.836 / 31.885 / 0.683 | 0.809 | 0.514 | 10.905 |
| 0 | 有 | 无 | 30.749 / 32.016 / 0.732 | 20.376 / 21.517 / 0.486 | 0.791 | 0.512 | 0.658 |
| 0 | 有 | 有 | 40.431 / 42.340 / 0.534 | 30.537 / 31.898 / 0.675 | 0.784 | 0.510 | 10.819 |
| 500 | 无 | 无 | 58.686 / 63.845 / 1.053 | 57.958 / 60.741 / 1.400 | 0.818 | 37.922 | 0.754 |
| 500 | 无 | 有 | 70.313 / 72.789 / 2.139 | 67.022 / 70.707 / 1.154 | 0.809 | 37.469 | 11.268 |
| 500 | 有 | 无 | 68.158 / 71.175 / 1.270 | 57.949 / 60.892 / 0.759 | 0.823 | 38.206 | 0.727 |
| 500 | 有 | 有 | 79.993 / 81.868 / 1.589 | 68.504 / 71.498 / 1.960 | 0.826 | 37.803 | 11.383 |

八个场景的中位数都没有超过既定回退阈值。非空 env 节省约 10–11 ms；空 env 的改善较小，
不将它描述成显著提速。大树仍主要耗时在通用 Core 捕获，变化渲染还包含原子持久化成本。
保留通用捕获符合本轮范围；缩小 runtime 加载应另行设计、验证配置和信任语义。

等待内存另测：Bun 替身写出 ready 后等待 0.4 秒，读取启动器 PID 的实际 RSS，每种情形
5 次取中位数。macOS 使用 `proc_pidinfo(PROC_PIDTASKINFO)`，这不是 peak RSS，也不包含
Bun/子孙进程的累计内存。单位 KiB：

| 额外文件 | env | 旧等待进程 RSS | 新等待进程 RSS |
| --- | --- | --- | --- |
| 0 | 无 | 2272 | 21024 |
| 0 | 有 | 19568 | 21312 |
| 500 | 无 | 2272 | 26032 |
| 500 | 有 | 19520 | 26272 |

旧空 env 路径 exec 到 Bun 替身；新路径多了约 18–23 MiB 的 debug Shine 等待进程。
非空 env 旧路径本来已有等待 Shine，新路径增加约 2–7 MiB。即使 runtime 已 drop，allocator
仍可能保留捕获时分配的页。本实现明确接受这项内存成本，以保证一个配置结果贯穿受记录
约束的准备及声明值解析；没有以进程数减少替代内存验收。发布构建、多实例、长任务和
硬件凭据场景需另测，不能从本基线推断其成本。

复现（按仓库工具链并关闭受限 sccache）：

```bash
rtk proxy env RUSTC_WRAPPER= mise exec -- cargo build --target-dir target
rtk proxy env RUSTC_WRAPPER= mise exec -- cargo test --lib --target-dir target live_bun_launch_performance_matrix -- --ignored --nocapture
```

## Linux 性能补测

使用已上传的同一 benchmark、同一个 Linux debug 二进制和相同 Bun 替身，沿用上面的
5 对热身、40 对交替样本、P95/MAD 和回退阈值。Clippy 完成后才开始采样；结果为热缓存，
没有硬件解密或业务执行。单位 ms：

| 额外文件 | env | 渲染变化 | 旧中位数 / P95 / MAD | 新中位数 / P95 / MAD | Config | Core 捕获 | 准备 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 无 | 无 | 14.293 / 14.941 / 0.270 | 14.089 / 15.077 / 0.307 | 1.174 | 0.694 | 0.780 |
| 0 | 无 | 有 | 18.504 / 25.976 / 0.433 | 18.449 / 20.112 / 0.458 | 1.157 | 0.673 | 4.664 |
| 0 | 有 | 无 | 24.647 / 27.616 / 1.077 | 14.711 / 17.859 / 0.608 | 1.240 | 0.715 | 0.869 |
| 0 | 有 | 有 | 28.506 / 29.852 / 0.631 | 18.517 / 19.687 / 0.426 | 1.193 | 0.739 | 4.761 |
| 500 | 无 | 无 | 70.426 / 75.948 / 2.153 | 70.890 / 75.281 / 2.199 | 1.323 | 55.810 | 1.019 |
| 500 | 无 | 有 | 74.722 / 85.907 / 2.065 | 74.852 / 80.657 / 2.153 | 1.352 | 56.882 | 5.128 |
| 500 | 有 | 无 | 81.880 / 86.866 / 2.043 | 71.893 / 77.628 / 2.133 | 1.282 | 56.205 | 1.018 |
| 500 | 有 | 有 | 85.840 / 92.462 / 2.199 | 74.594 / 79.868 / 2.086 | 1.299 | 56.514 | 5.114 |

未发现超过既定阈值的延迟回退。非空 env 的中位数减少约 10–11 ms；空 env 在大树下略增，
差值仍小于既定噪声阈值。大树通用捕获约 56 ms，继续是主导成本。不能把两台不同机器的
绝对耗时直接比较成 OS 性能差异。

等待 RSS 使用 `/proc/<pid>/status` 的 `VmRSS`，其余方法与 macOS 相同，单位 KiB：

| 额外文件 | env | 旧等待进程 RSS | 新等待进程 RSS |
| --- | --- | --- | --- |
| 0 | 无 | 1920 | 29240 |
| 0 | 有 | 27652 | 29528 |
| 500 | 无 | 1920 | 32892 |
| 500 | 有 | 27660 | 32448 |

Linux 的空 env 路径新增约 27–30 MiB debug 等待 RSS，非空 env 增加约 2–5 MiB。
这是同一进程内捕获后 allocator 可能保留页的实测成本，沿用前述明确取舍；未承诺减少内存。
本轮没有为降低数值而改变构建模式、跳过通用捕获或扩大受管/信任边界。

定向测试、两次全量日志、Clippy 和性能日志已取回。用户另行批准额外脚本上传后，执行了
Python 标准库驱动的真实 Bun 1.3.14 控制终端检查，6 项全部通过：

- 对空 env 与 `TOKEN=ALIAS` 两种安装记录，验证空参数、字面量 `--`、中文参数、cwd、
  安装目录绑定、项目及 env override 分层、别名显式值和未声明时的继承值。
- 使用 `pty.fork()` 创建控制终端，确认 Shine 为前台进程组，Bun 在同一组、stdin 为 TTY；
  写入终端输入后 Bun 收到相同文本，Shine 返回 Bun 的退出码 42。
- 向终端写入 `0x03`，由终端驱动产生 SIGINT；Shine 返回 130，直接 Bun 子进程消失，
  不遗留等待子进程。测试不通过直接 `kill(SIGINT)` 替代 Ctrl-C。

脚本为 `/private/tmp/shine-live-bun-linux-pty.py`，远端副本为隔离目录下 `real-bun-pty.py`；
日志已取回 `native-real-bun-pty.log`。运行时使用已验证的 Linux debug 二进制，Bun 指向
固定 1.3.14 安装路径，临时 fixture 自动清理；未读取真实凭据或修改实例现有配置。
脚本 SHA-256：`fc8b5ffe8962a7dcf613e86a46b8034170505babd1a55c46e0def00847cc85eb`。
这些检查不覆盖硬件解密交互；macOS/Windows 补测见下文。

## Windows 原生验证

用户授权通过 `nuc.win.local` 上传源码和受控测试文件，在独立临时目录执行。
Windows 11 x64（build 22631），MSVC / Visual Studio 2022 Build Tools，Rust 1.96.0、
Bun 1.3.14；32 个逻辑 CPU、约 64 GiB RAM。使用受控 Bun 替身和隔离状态，未激活
实例现有 preset。初始归档 SHA-256：
`7e0d64384ec34fe334b9e825aa04984982607c57acc180a00ccb241b49325891`。
之后同步了栈修复与测试目录修正，以下结果基于修正后的源码。

- `live_bun` 定向测试：10 项通过（4 项实际 CLI 进程测试，6 项 Core/receipt/模板测试）。
  覆盖 CMD、PowerShell、特殊配置路径、空参数和 Unicode、项目覆盖、缺失/损坏密钥、
  Bun 缺失及退出码；Windows 双 shim 的旧格式批准升级、故障恢复、重试和卸载也通过。
- 全 workspace/all-features：1259 项单元/集成测试通过，6 项文档测试通过，1 项失败。
  失败为 `completion::tests::embedded_candidates_include_known_categories`：
  `cli/src/completion.rs:639` 预期 Windows 不含 Surge artifact 补全，实际仍包含。
  单独精确复测同样失败；补全模块及 App metadata 模块未被本次修改。
  未另行构建整个修改前 Windows 基线，不将其描述成基线二进制复现。
- 严格 Clippy 未通过：Core 报 9 项既有 Windows 条件编译问题，包括 launcher 的未使用
  `host` 和测试 helper、`runtime/host.rs` 的 needless return，以及旧恢复测试的 needless
  borrow。失败涉及的代码行与 HEAD 相同；未为通过本次检查而全局屏蔽警告。
- 初始实际 CLI 安装出现 `3221225725`（stack overflow），仅 heap-box 外层 future 不足以
  修复深层生命周期调用。最终采用 MSVC 链接参数 `/STACK:8388608`，实际二进制 PE
  header 确认 8 MiB 预留，原生安装及启动测试通过；临时 heap-box 方案已撤回。
  测试同时显式创建 HOME、APPDATA、LOCALAPPDATA 对应目录，避免 Windows Known Folder
  解析隔离目录失败。修正后本地全量 Nextest、严格 Clippy 通过。

Windows 的性能基准尚未测量；当前 benchmark 使用 Unix 进程/内存接口。上述原生功能
套件使用 Rust Bun 替身；额外真实 Bun 检查见下文，硬件解密仍未覆盖。

## macOS 与 Windows 真实 Bun 终端补测

用户授权后，使用 Bun 1.3.14 和当前平台 debug Shine，在隔离安装目录执行受控 TypeScript
探针；所有环境值均为合成值，未激活已有 preset 或读取真实凭据。

macOS arm64 沿用 Linux 的 `pty.fork()` 控制终端方法：空/非空声明 env 各测试参数及配置、
终端输入和 Ctrl-C，共 6 项通过。输入后退出 42；写入 `0x03` 后 Shine 返回 130，Bun
子进程已被回收。初次检查的 cwd 字符串因 macOS 临时路径的符号链接拼写不同而失败，
将测试根目录规范化后复测通过，未修改产品 cwd 行为。
脚本 SHA-256：`c7bc8aa12b38ef8346a337dd02d1646042dfcf63be17ce003e53604786136e85`；
日志为本地 `shine-live-bun-macos-pty.log`。

Windows 11 x64 使用 PowerShell/C# 调用原生 ConPTY API，独立输入/输出 pipe 配合
`STARTUPINFOEX` 创建 CMD 和 Windows PowerShell 5.1。每种 shell 对空/非空声明 env 分别
测试参数与配置、终端输入和 Ctrl-C，共 12 项通过。额外 2 项直接 Bun / PowerShell Ctrl-C
对照也通过。Bun 明确报告 stdin 为 TTY，cwd、空参数、字面量 `--`、中文参数、配置绑定、
项目/override 分层和 alias 均符合预期；输入退出 42。Ctrl-C 后验证 Bun PID 和其父 Shine
PID 均不存在，检查发生在关闭 ConPTY 之前；不是终端关闭或测试清理阶段代为杀死进程。

Windows 外层 CMD 的 Ctrl-C 状态为 255，PowerShell 5.1 为 0；PowerShell 直接调用真实
Bun 的对照同样为 0。这里记录原生外层 shell 的状态，不宣称等于 Unix 的 130，也不宣称
PowerShell 的 0 表示业务成功。ConPTY 写入 `0x03`，未以直接杀进程替代 Ctrl-C。
测试宿主先清除 SSH 重定向标准句柄的继承并恢复 Ctrl-C 处理，再创建 ConPTY 客户端，
避免验证成普通 pipe，或继承宿主的 Ctrl-C 忽略属性。
这些行为依据 Microsoft 的 [标准句柄说明](https://learn.microsoft.com/en-us/windows/console/getstdhandle)
和 [Ctrl-C 继承说明](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler)。

实际补测发现 PowerShell 5.1 的原生调用丢失空参数和字面量 `--`；已修复新格式模板，
使用显式 Windows argv 编码及 `ProcessStartInfo`，禁用 shell execution 并继承 stdio/cwd。
原 legacy 模板未修改；未发布的 `live-bun-v2` 模板修正后，Windows 的 10 项定向套件
重新通过。PowerShell 回归测试新增空参数、`--`、空格、嵌入双引号、末尾反斜线及
引号前反斜线的逐参数断言。macOS 的 15 项定向测试与严格 Clippy、双语网站 locales/
typecheck/生产构建也通过。

Windows 脚本 SHA-256：`9d8bc9577a709b9db06d6bcf4c38978b679abb4a6ea8e0eace244bf53646d158`；
`native-real-bun-conpty.log` 与 `native-focused-powershell.log` 已取回。
Windows 全量套件与 Clippy 的上述未修改代码失败仍保留，不将定向复测描述成全量通过。

## 尚未验收的边界

- Windows 全量套件与严格 Clippy 的上述失败；Windows 性能。
- 硬件/插件解密交互、冷缓存和发布构建性能。
- 释放锁后共享 rendered 文件仍可被其他调用或 lifecycle 修改，Live 依赖仍可变。
  不保证 Bun 消费本次捕获的确切字节，不引入长时间持锁或执行快照。
