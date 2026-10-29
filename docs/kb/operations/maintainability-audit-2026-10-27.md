# Shine 可维护性与精简审视

> 审查已于 2026-10-28 关闭，原 B1–B4 及后续全部候选冻结，恢复正常功能开发。
> 当前决定以[审查关闭结论](architecture-necessity-review-2026-10-27.md)为准。
> 本文统计、候选建议和实施批次保留为历史证据，不构成当前待办或实施授权。

审视基线：`release` 的 `bd651fd4eaa82200bc06545c133ce9339de248ca`，917 次提交；最近稳定标签为
`v2.7.1`（`ce3fd418`）。本报告记录当时的审视结论和实施批次，**没有实施下列 Rust 重构**。
它是该基线的决策证据，不是持续更新的代码规模仪表盘。

## 结论

- 181,144 行 Rust 中，生产部分为 **93,910 行**，测试/夹具/支持部分为 **87,234 行（48.2%）**。
  不能把全部 18 万行当作生产实现，也不能把测试规模直接当作冗余。
- 全部 460 个受跟踪 Rust 文件均能从 crate/build/integration-test 根通过模块声明到达；
  本轮**没有确认可直接删除的大块死代码**。仅测试调用、公开重导出和平台替代实现不等于无用。
- 此前若干重构确实减少了生产实现，但新增测试抵消了减量；另一些抽取同时增加了生产与测试成本。
  应逐项保留有效规则收敛、精简支持代码，而不是整体回退历史重构。
- 初筛曾提出四个独立的小批次（现已冻结）：退出状态处理、Action 权限构造、Shell rendered 删除选择、
  Core 测试交互对象。预计收益是**百行量级**，没有足够证据承诺数千行或固定比例的安全减量。
  测试补强计入成本，最终必须按实际 diff 验收。
- 不推荐通用生命周期/事务引擎、不推荐合并三种 journal 的领域语义，也不推荐为文件变短继续拆模块。
  这些方案会跨越审批、资源所有权或恢复边界，当前没有证明能降低总维护成本。

## 统计口径与覆盖范围

统计覆盖 460 个受 Git 跟踪的 Rust 文件；不统计依赖目录、`target/` 或未跟踪材料。
本页保留统计方法、领域汇总和影响取舍的历史抽样；逐文件行数与测试行区间属于可重新计算的基线快照，
不再作为长期附件维护。原始账本的历史位置见[评审追溯说明](architecture-necessity-review-2026-10-27.md#证据的阅读与追溯)。

计数使用 `syn 2.0.119` 解析源码，追踪普通/内联模块及 `#[path]`，将只有 `test` 启用时才成立的
`cfg` 条件沿模块树传播。物理行包含注释和空行；空白分隔行归入前一部分。平台条件取源码并集，
不进行宏展开，不把字符串中的示例程序当作独立源码。另按实际用途把无条件编译的 `test_support`、
`Config::new_for_test`、`Session::parsed_files`、`InMemoryHost::queue_process_output` 归入测试支持。
`runtime::fixture` 和大部分 `InMemoryHost` 服务于正式的 `preset test`，仍计作生产代码。

| 范围 | Rust 文件 | 生产行 | 测试/支持行 |
| --- | ---: | ---: | ---: |
| CLI 源码 | 202 | 37,838 | 33,554 |
| Core 源码 | 236 | 55,993 | 34,632 |
| CLI 集成测试 | 19 | 0 | 18,054 |
| Core 集成测试 | 2 | 0 | 994 |
| build.rs | 1 | 79 | 0 |
| 合计 | 460 | 93,910 | 87,234 |

另外，基线有 311 个受跟踪 Markdown/MDX 文件，共 49,273 行；该口径不包含 JSON 证据、图片等。
宏展开前的生产语法项包括 380 个模块声明、531 个 struct、228 个 enum、1,827 个普通函数和
967 个 impl 方法。这些是源码导航指标，不是复杂度得分。单表达式函数可能承担能力收窄、错误映射或
兼容职责，因此不以其数量直接推断“多余转发层”；各候选的调用层变化在下文单独说明。

**覆盖深度**：所有文件完成语法、模块归属、测试分界、函数体重复和跨文件重复片段扫描；
下表各域另检查生产入口、已有共享规则及相关安全边界。候选项追踪消费者和测试。
这不是逐行形式化验证、完整类型解析调用图或动态覆盖率分析；没有运行全部功能来证明不存在其他死代码。

| 审视域 | 文件 | 生产/测试支持行 | 语义检查入口及结论 |
| --- | ---: | ---: | --- |
| CLI 路由、补全 | 25 | 3,327 / 2,484 | `dispatch`、`commands`、`completion`：保留配置初始化前路由、选择范围与 Clap 回调 |
| CLI App/Shell/Sys 适配 | 35 | 5,268 / 7,635 | `apps/install`、`uninstall`、三个 recovery：审视 C06；保留预览和权限差异 |
| CLI review、展示 | 30 | 6,495 / 4,822 | `lifecycle_plan`、`presentation`、`list`：保留展示与原始身份分离；不合并全部 renderer |
| CLI 配置、信任、来源 | 11 | 3,596 / 3,706 | `config/env_resolution`、`env_write`、`trust`、`git_pull`：读快照与有锁写入不是重复实现 |
| CLI 环境与密钥 | 36 | 8,529 / 5,684 | `workspace`、`cache`、`local_cache`、`secret`：C01/C05；不统一普通/混合缓存及后端校验 |
| CLI SSH | 17 | 3,094 / 2,591 | `ssh/mod`、`agent`、`broker`：C01；会话清理、传输和解密授权仍分别拥有流程 |
| CLI 预设工具 | 15 | 2,754 / 2,765 | `preset_*`：复用 Core 入口和 `preset_report`；保留各报告结构与诊断 |
| CLI 维护、自更新 | 19 | 3,083 / 1,861 | `maintenance`、`update_check`、`self_install`：后台建议与强制补丁门禁不同，不合并调度器 |
| CLI 其他与支持 | 14 | 1,692 / 2,006 | `task`、`proc`、`platform`、`shim`、`test_support`：C01/C10；保留选择歧义和环境恢复 |
| Core 契约与原语 | 28 | 5,423 / 3,163 | `action/contracts`、`plan`、`persist`、`install`：C02；稳定 DTO、序列化及原子写职责保留 |
| Core 分析 | 3 | 1,672 / 1,254 | `analysis/mod`、`syntax`：绑定身份、预算、语法缺口不同于运行时 admission；保留 |
| Core Frontend | 9 | 1,886 / 1,633 | `read_only`、`trusted`、`conformance`：能力收窄不是空转发；C04 仅共享测试对象 |
| Core 计划 | 43 | 8,231 / 11,858 | `planner`、`approved`、`shell_removal`：C02/C03/C04；审批后重读不能删除 |
| Core 事务与恢复 | 52 | 13,699 / 3,434 | 三个 executor、journal IO、App commit/recovery：C07/C08；领域状态机保留 |
| Core App | 23 | 5,085 / 3,363 | `assessment`、`upgrade`、`refresh_selection`、`managed_json`：已有共享 owner 保留；C04 |
| Core Shell/launcher | 16 | 6,256 / 2,666 | `delivery`、`lifecycle/remove`、`templates`、`profile`：C03；旧 launcher 字节重建保留 |
| Core Sys | 27 | 5,823 / 1,591 | `sys/lifecycle`、`sys_bootstrap`、`sys_profile`：managed/bootstrap/profile 的状态和权限不同 |
| Core 预设工具 | 13 | 3,977 / 1,487 | `validation/source`、`authoring`、`fixture`、`lint`、migration：已共享源捕获；正式模拟入口保留 |
| Core Host 与副作用 | 5 | 1,969 / 485 | `host`、`memory`、锁、进程、`privileged_files`：保留原生路径与平台原语差异 |
| Core 组装与 admission | 11 | 1,972 / 1,942 | `bootstrap`、`preset`、`trust`、`code_analysis`、`command_detection`：C10；不混淆展示与执行证据 |
| Core 跨域测试 | 6 | 0 / 1,756 | `lifecycle_concurrency`、`refactoring_contracts`、`privileged_contracts`：C04；保留独立风险覆盖 |
| 集成测试 | 21 | 0 / 19,048 | 进程、退出、配置、预设、发布包测试：C09；不将真实进程测试改成纯函数测试 |
| build.rs | 1 | 79 / 0 | 嵌入资源重建、Git 信息、Windows 栈设置：有实际构建职责，保留 |

## 历史重构复核

以下按每个提交的 parent→commit 实际增删行归类，非仅按路径是否带 `tests` 计数。
`+/-/净` 分别表示新增、删除、净变化；文档为受跟踪 Markdown/MDX。搬移也计入增删，净值才表示规模变化。

| 提交 | 生产 + / - / 净 | 测试支持 + / - / 净 | 文档 + / - / 净 | Rust 净值与结论 |
| --- | --- | --- | --- | --- |
| `c431aeb8` | 254 / 285 / -31 | 676 / 26 / +650 | 94 / 6 / +88 | +619；保留 refresh/source/literal 规则收敛，不能把大量新增覆盖记成精简收益 |
| `6e4bbd52` | 133 / 122 / +11 | 616 / 111 / +505 | 72 / 1 / +71 | +516；保留 App/Sys 特权原语单一 owner，承认没有行数收益 |
| `56b1782d` | 32 / 44 / -12 | 356 / 1 / +355 | 47 / 2 / +45 | +343；保留 manifest 版本规则收敛，检查支持代码而非删除各 reader 边界 |
| `d095431b` | 32 / 54 / -22 | 236 / 0 / +236 | 50 / 1 / +49 | +214；保留五条恢复路径的有序观察规则，不再增加平行观察框架 |
| `914b8ca4` | 373 / 575 / -202 | 55 / 22 / +33 | 85 / 64 / +21 | -169；保留已实现的 App 编排简化 |
| `58b621c8` | 11 / 19 / -8 | 64 / 101 / -37 | 87 / 5 / +82 | -45；保留旧测试适配器退役，迁移断言仍落在生产 owner |

对应决策：[ADR 0141](../decisions/0141-shared-refresh-literals-and-source-validation.md)、
[0144](../decisions/0144-shared-app-recovery-observations.md)、
[0145](../decisions/0145-domain-owned-manifest-schema-normalization.md)、
[0146](../decisions/0146-shared-file-primitives-and-test-fixtures.md)、
[0139](../decisions/0139-production-app-inspection-and-shell-profile-tests.md)。
当前源码仍使用这些共享 owner；特权测试中的暂停恢复/取消探针与最终文件状态断言覆盖不同风险，不能互相替代。
没有建议整体或局部回退上述六项实现。

稳定标签 `v2.7.1` 到本基线另有 Windows split-DNS 所有权修复：`host.rs`、`planner/sys_support.rs`
及相关测试均已纳入本轮当前态审视。后续精简必须保留 expected receipt content 的观察输入；
不能拿标签版本覆盖该修复。历史提交的分项数字也不代表该模块到今天的全部演进成本。

## 候选清单与取舍

源码行号均指上述基线；实施前重新确认。下表预算是根据现有代码块和拟定改法估计的区间，不是已完成的 diff。
“测试”包含支持代码；“文档”是同步既有 KB 的预留净增量，不要求每批另写 ADR。
没有把被拒绝候选、API 隐藏、移动代码或删除独立测试算作收益。

| ID / 优先级 | 决定 | 生产预计新增/删除 | 测试预计新增/删除 | 文档净增 | 规则与调用结构变化 |
| --- | --- | --- | --- | --- | --- |
| C01 / P1 | 合并退出状态处理 | 18–24 / 45–50 | 10–25 / 0 | 0–8 | 规则 3→1；两个 wrapper 删除；task 增加一层短 helper 调用 |
| C02 / P1 | 合并同型权限构造 | 45–75 / 125–175 | 20–50 / 0 | 0–10 | 成对 Write/Remove 构造归一；每分支增加一层纯 helper，无新 DTO |
| C03 / P2 | 合并 rendered 删除选择 | 30–40 / 45–65 | 10–25 / 0 | 0–10 | 选择规则 2→1；两个调用方各增加一层纯函数，无 IO/权限能力 |
| C04 / P1 | 合并四个同语义测试交互对象 | 0 / 0 | 28–36 / 90–105 | 0–8 | 四个测试类型变一个；生产调用层不变，所有场景/断言保留 |
| C05 / P3 | 暂缓 recipient 清理小合并 | 本轮预算 0 | 本轮预算 0 | 0 | 两个 7 行函数相同，但净收益不足，且容易扩成错误的后端校验统一 |
| C06 / P3 | 保留 CLI 生命周期编排 | 0 | 0 | 0 | 不为两段相似分支引入操作模式、observer 适配或统一返回包装 |
| C07 / P3 | 保留分域 journal IO | 0 | 0 | 0 | 三个 load、三个 save 各自保留验证和错误上下文 |
| C08 / P3 | 保留 App commit/recovery 分派 | 0 | 0 | 0 | 不以 wildcard 或通用执行接口换取少写 enum 分支 |
| C09 / P3 | 暂缓统一集成测试 Fixture | 0 | 0 | 0 | 环境、配置发现、短 socket 路径、子进程释放差异留在场景 owner |
| C10 / 排除 | 保留能力/兼容/平台适配 | 0 | 0 | 0 | 有契约的桥接层不作为死代码 |

C01–C04 的合计预算为生产新增 93–139、删除 215–290 行；测试支持新增 68–136、删除 90–105 行；
Rust 合计预计净减少 **30–234 行**，文档净增加 0–36 行。区间是实施前预算，不是交付承诺。
若真实改动和所需测试超出预算，重新核算；不能削弱覆盖以达成负数。没有为整库推算额外“潜在节省”。

### C01：退出状态归一

证据：[workspace](../../../cli/src/env/workspace.rs) 的 `finish_command_status`（164–178）、
[SSH](../../../cli/src/ssh/mod.rs) 的 `finish_ssh_status`（414–428）函数体 token 完全相同；
[task](../../../cli/src/task/mod.rs) 的 `execute_task` 尾部（207–220）重复同一规则。
三处均为成功返回 `Ok(())`，否则转交退出码，Unix 信号映射为 `128 + signal`，其他无退出码情况返回 1。

改法：在现有 `cli/src/proc.rs` 放置 `pub(crate) fn finish_command_status(ExitStatus) -> Result<()>`，
直接迁移一份现有实现；删除两个旧 helper，所有 env/live-Bun/proxy/file-group/run、SSH 和 task 消费者直接调用。
不保留旧路径转发，不改变公共 API，不创建新模块、策略枚举或进程运行器。

必须保留各调用点的位置：SSH 已完成 listener/task/context 清理，workspace 已完成敏感临时文件清理后才交付状态。
共享函数不持有 child、信号转发器或清理 guard；不能提前 `process::exit`。
生产规则 3→1；收益主要是未来修改退出语义只改一处，不是删除进程边界测试。

### C02：Action 权限构造归一

证据：[managed_file](../../../core/src/action/contracts/managed_file.rs)、
[json](../../../core/src/action/contracts/json.rs)、[shell](../../../core/src/action/contracts/shell.rs)、
[sys](../../../core/src/action/contracts/sys.rs) 中有 18 处 `(access, path)` 循环。
其中多处对每个路径依次插入 Write/Remove，例如 managed-file:201、JSON:129/147、Shell:219/253/269/314/332、Sys:41。
消费者是 `ActionIrV1::permission_requirements`，不是带 purpose/target 归属的 planner accumulator。

改法：在现有 `action/contracts.rs` 增加一个私有 `add_write_remove_permissions`，接收现有
`PermissionSetV1`、`path_identity` 回调和按原顺序排列的路径切片。每条路径仍依次调用两次 identity 回调并插入
Write、Remove，不能因为路径相同只调用一次。替换恰好等价的循环；不使用 macro、布尔模式或通用策略 trait。

只含 Remove 的 destination、可选 rollback/backup、previous_present 条件和 Administrator 判定仍留在原分支。
不把 18 处循环全部改成同一个权限套餐；不与 planner 的 presentation-purpose 权限构造合并。
保持集合、不可计算诊断、序列化及 fingerprint 不变。

### C03：Shell rendered-file 删除选择归一

证据：[规划](../../../core/src/runtime/planner/shell_removal/rendered.rs) 的 `plan_rendered_shell_removal`
与[执行准备](../../../core/src/runtime/shell/lifecycle/remove.rs) 的 `prepare_shell_rendered_removals`
均从 receipt 选择 rendered 路径、排除 managed root 之外的路径、查找所有消费者，并保护未被选择的消费者。
目前维护同一选择规则需要改两处。**两处后续 Host 观察顺序并不相同，不属于可合并部分。**

改法：在现有 `runtime/shell.rs` 增加 runtime 内可见的纯函数
`rendered_removal_groups(manifest, selected_keys, rendered_root)`。
输入为 `&ShellManifest`、借用的 category/command 键集合和 `&Path`；返回按路径排序的
`BTreeMap<PathBuf, Vec<&ShellManifestEntry>>`。只返回 root 内、全部 receipt 消费者均被选择的组；
组内保持 manifest 顺序，因此首个 target 不变。调用方分别从各自新加载的 manifest 构建输入。

规划方保留原来的状态捕获、错误优先级、Plan step/permission 顺序；执行方保留 approved/dry-run 门禁，
先查 rollback、再查 destination、再读字节的顺序以及自己的 receipt projection。
不把规划输出传给执行，不缓存审批前观察，不改变路径身份编码或锁的持有范围。
若 helper、键集合适配和必要测试抵消本项收益，则本批暂缓，不扩大范围“摊薄”成本。

### C04：Core 测试交互对象归一

证据：四个对象都原样返回 confirm 的 default、批准 admin、原样返回 select_many 的 defaults：
[App lifecycle_tests::Interaction](../../../core/src/runtime/app.rs)（1065 起）、
[planner::tests::Interaction](../../../core/src/runtime/planner/tests.rs)（108 起）、
[Frontend conformance::Human](../../../core/src/frontend/conformance.rs)（20 起）、
[lifecycle_concurrency::Interaction](../../../core/src/runtime/tests/lifecycle_concurrency.rs)（13 起）。
它们拥有相同语义，修改默认测试交互需改四份实现。

改法：在现有 `runtime/tests/mod.rs` 定义一个无字段的 `DefaultAllowAdminInteraction`，
经 `runtime/mod.rs` 的 `#[cfg(test)] pub(crate) use` 供这四处使用。直接导入或 alias；
App 的 assessment_tests 也直接导入共享对象，不保留依赖其他测试模块私有类型的链。
不改变生产 `RuntimeInteraction` trait 的默认方法，不增加生产导出或跨 crate 测试依赖。

**不合并** NoAdminInteraction、DenyAdmin、计数/记录授权参数、挂起或 panic 的探针；
`refactoring_contracts` 的禁止提权断言、refresh/bootstrap 的调用次数断言、集成测试中的拒绝策略全部保留。
自动扫描找到的 14 份相同 `select_many` 方法，只有上述四个完整对象满足合并条件。
测试条目、请求、fixtures、预期状态和断言一项不删，因此不需用“新的共享对象单元测试”复制这些消费者测试。

### C05–C10：明确保留或暂缓

| 项目 | 源码证据与不实施原因 |
| --- | --- |
| C05 recipient 清理 | `env/commands.rs:494` 与 `env/workspace.rs:115` 都 trim 并去空项，保留顺序与重复项。共享到 `env/mod.rs` 仅少数行收益；GPG/age 的有效性、空白接受及 workspace/global fallback 不同，不能借机统一后端策略。本轮暂缓 |
| C06 CLI 编排 | App install/uninstall 的 review→prepare→execute 结构相似，但 category 元数据、observer、preview、purge 和完成状态不同。App recovery 还在 prepare 前单独请求 admin；Shell/Sys 没有同样一步。保持现有 `execute_reviewed`，不再叠加 dispatcher |
| C07 journal IO | 三个 executor 的 `journal_io.rs` 共 161 行，还各有 optional-read/marker/remove 等不同工具。load/save 都验证领域 DTO、保留原始 bytes、使用各自错误上下文。泛型 IO 层需要额外 validate 能力、文件名/错误参数和包装函数，尚无总量下降证据。保持分域，拒绝统一 schema/恢复流程 |
| C08 App 分派 | `commit/action.rs`、`recovery_plan.rs`、`recovery_apply.rs`、`receipts.rs` 的非 App enum 列表重复，但结果分别是错误、Blocked Plan 或 receipt 判定。现有 `AppCommitAction` 已是 commit 专用借用视图；扩成所有阶段的视图会增加字段搬运。保留穷尽检查，不能改 `_ => ...` 隐藏新增 variant 的审查义务 |
| C09 进程 Fixture | `cli_dispatch`/`list_views` 有同型 HOME 设置，但前者故意不初始化 state，后者写 config/cache；`env_configuration` 测 CLI `--config-dir` 并移除环境覆盖；SSH 故意用短 `/tmp` socket 路径；workspace export 管理活 child。暂不引入带开关的统一 Fixture。只有 11 行路径装饰函数完全相同，单独抽取不足以改变维护成本 |
| C10 PATH 与兼容桥 | `proc::ensure_command` 只加 `.exe`；`platform::command_exists_on_path` 是诊断并加 `.cmd/.bat`；Core command detection 还检查模式、symlink、captured PATH 和额外目录。不能统一。`persist`/`sentinel`/`bin_links` 等公开重导出保留兼容；ReadOnlyFrontend 的错误投影与能力收窄有职责 |

死代码检查还排除了以下误报：`Config::new_for_test` 支持跨 lib/bin 测试；`parsed_files` 被
`tests/builtin_presets/analysis.rs` 用于缓存复用断言；`queue_process_output` 被 Core 集成测试用于进程结果注入；
`ReadOnlyFrontend::request_review` 被 conformance 测试验证能力等价；`PresetSnapshotBuilder::overlay_file`
是测试常用的无 executable 标记便利入口。把它们 `cfg(test)` 化可能破坏 integration-test 编译，
删除便利入口也会把参数噪音转移到调用方。本轮无删除建议。

旧 manifest/launcher 兼容、migration、Windows/macOS/Linux 分支及 legacy service 清理均有现有生产职责。
`#[allow(dead_code)]` 出现在测试声明/非 Windows 的特定字段上，也不是遗留实现的充分证据。
编译器、文本引用计数和 clone 扫描都只提供候选；本轮未获得可以绕过这些契约删除代码的证据。

## 可执行批次与验证

本轮原计划中的“先删除死代码”阶段结论为无确认项，不为凑批次而删除。后续推荐按 B1→B2→B3→B4 执行；
四批无代码依赖，顺序体现验证成本，任何一批可独立撤销。不要把无关缺陷修复或格式搬移夹入精简。

| 批次 | 改动与停止边界 | 必须保留/补充的消费者验证 |
| --- | --- | --- |
| B1 = C04 | 只替换四个同策略测试交互对象和直接 imports；记录类型 4→1、测试条目数量不变 | App lifecycle/assessment、planner、frontend conformance、lifecycle_concurrency 原有测试；特别保留 NoAdmin/计数/挂起对象，不能用共享对象代替 |
| B2 = C01 | 只移动一份退出状态规则并迁移调用点；不改变进程启动、信号转发、guard 或 cleanup | `ssh_nonzero_exit_cleans_session_before_propagating_status`；task 的真实 CLI 退出测试；`live_bun_preserves_non_utf8_arguments_and_signal_exit_mapping`；file-group 的非零退出与清理。用现有 SSH/task fixtures 补 Unix signal 场景，检查清理先于退出；Windows 原生保留非零码 |
| B3 = C02 | 只替换 Write/Remove 同型构造，不碰 planner accumulator/权限决策 | `action::tests` 的精确权限断言、`resource_contracts_preserve_exact_permissions_and_reject_other_rollback_kinds`、App factory 权限/顺序契约。补所有被改分支的集合断言缺口，覆盖 optional original/backup/previous_present 和 admin，仍保持不兼容分支 fail-closed |
| B4 = C03 | 只共享纯选择；预算超出则暂缓该批 | `rendered_uninstall_preserves_a_file_with_an_unselected_consumer` 现仅检查 Plan，需在同一 fixture 补 approved uninstall 后 sibling 文件/receipt 保留；复用 rollback-occupied、shell_transactions、rendered recovery 测试。覆盖全选/部分选、root 外路径、多路径顺序、缺失目标、非普通文件、审批后 receipt 改动 |

实施前在原实现运行对应消费者测试，记录原有失败；新增缺口场景先在原实现通过再抽取。
不新增并长期保留一份旧实现作为测试 oracle，不把进程测试降成 helper 测试，不把多个独立故障场景塞进一个长测试。

后续验证命令使用 RTK 和 mise，所有构建加 `--target-dir target`。每批先运行相关过滤测试；
最终整合时运行下列 Rust 检查及适用 hooks，之后只因新改动、失败或未解疑点重跑：

```bash
rtk proxy mise exec -- cargo fmt --check
rtk proxy mise exec -- cargo check --workspace --all-targets --all-features --target-dir target
rtk proxy mise exec -- cargo clippy --workspace --all-targets --all-features --tests --benches --target-dir target -- -D warnings
rtk proxy mise exec -- cargo nextest run --workspace --all-features --target-dir target
rtk proxy mise exec -- cargo test --workspace --doc --target-dir target
rtk proxy mise exec -- cargo deny check bans licenses sources
rtk proxy mise exec -- typos
rtk git diff --check
rtk git status --short
```

文档测试单列，以保留 Host/ReadOnlyFrontend 的 compile-fail 能力边界。原生 macOS/Linux/Windows 测试要求来自
[现有 CI](../../../.github/workflows/test.yml)；不能以本机或交叉编译代替。实际 SSH 远端、管理员设施和密码硬件
未在本轮验证，也不因本报告自动获得验证结论。任何未来本地 CLI 探针必须使用独立 `SHINE_CONFIG_DIR`。

最终验收同时核对生产/测试支持/文档的实际增删、每个规则的维护位置、类型/调用层变化，以及必要断言的去向。
四批累计 Rust 必须净减少；若因必要覆盖而不能达成，应如实报告并重新取舍，不得通过删独立测试、压行或移出仓库达标。
对外命令、API、序列化、Plan 指纹、原始 bytes、锁和恢复边界不变；预期无公共手册行为变更。
实施仅同步受影响 KB；不需要为纯支持代码共享新增大型架构说明。

## 本轮交付与验证边界

本轮只增加本报告和静态分类账本，并在 KB 索引增加入口；Rust 生产与测试代码均未改动。
已完成 AST 解析、全部模块路径解析、分项账本校验、重复函数/片段初筛、候选消费者及历史增删复核。
文档相对链接、JSON 行区间/总量/文件集合一致性、拼写、空白/换行和 `git diff --check` 检查通过。
未运行项目 Rust/Bun 测试、Clippy、原生跨平台或外部系统验证；内部文档交付不借此宣称重构已通过验收。
临时解析工具构建最初受环境中的 sccache 沙箱权限限制，单次命令清空 `RUSTC_WRAPPER` 后构建成功；
没有修改仓库工具链或用户配置。临时探针不纳入生产依赖或常驻测试框架。

原有 `.pnpm-store/` 及三个未跟踪草稿未改动。没有提交、推送或发布。
