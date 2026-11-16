# `shine sys init` 独立刷新初始化配置 PRD

> **状态：设计方向已确认，尚未实现。** 2026-11-16 根据当前仓库调研整理。
> 本文只定义独立刷新入口，不授权代码实现或发布。基线为 `release` 分支 `12d88f9e`。
> 本文为内部设计文档，不进入公共站点。

## 1. 产品定义与现状

`shine sys init` 独立重新生成并接入当前 Sys preset 的 shell 初始化配置，使后续 shell
启动持续加载，无需选择或安装软件。

现有 `base.pre.sh`、`base.post.sh` 已经能够表达环境变量、函数、别名、键绑定和其他启动
代码。Bash/Zsh 共用文件时，作者可以通过 `BASH_VERSION`、`ZSH_VERSION` 分支提供不同
配置，不要求功能或顺序一致。例如，Bash 可以使用 `set -o vi`，Zsh 可以使用 `bindkey -v`。
Shine 不拆解、筛选或调度文件中的功能。

目前选择一个已安装的软件执行 bootstrap，也能触发 profile 重建；但空选择会提前返回，
无法单独完成此操作。因此新增命令解决的是**独立刷新入口缺失**，现有文件已经具备配置
表达能力。

依据：[bootstrap 批处理](../core/src/runtime/sys_bootstrap.rs)、
[profile 组合器](../core/src/runtime/sys_profile/compose.rs)。

## 2. 目标与范围

目标：

- 不选择软件，即可应用当前完整初始化配置。
- 没有任何软件执行记录时，也能应用 base 文件。
- 保留已启用软件集成，不重新检测或安装软件，不改变启用状态。
- 重用现有文件布局、目标 shell 判定、组合顺序、托管路径和修改保护。
- 遵守现有信任、权限、快照审批和所有权边界。

不包含：

- `sys init <功能名>`、配置项目、依赖图、项目启停状态。
- 成功记录驱动的初始化任务执行。
- 新的 shell 文件组织方式、能力对齐或脚本语法转换。
- 自动加载当前终端、安装软件、改变默认 shell。
- zsh-vi-mode 的 Bash 筛选修复。
- 通用脚本执行系统或新的事务恢复系统。

本版不引入 shell 专属目录、文件优先级与回退、`--shell`、新 shell 检测政策、文件布局
迁移、无 item 的事务 journal 或双阶段原子发布。必要新增仅包括独立命令及其快照审批
入口，以及不依赖软件 item 的外部 base 代码授权边界。不新增运行记录、启停状态或状态
文件；用户显式授予持久信任时仍使用已有 trust store。

## 3. 命令与用户流程

基本用法：

```bash
shine sys init --dry-run
shine sys init
```

只提供现有 Sys 操作已经使用的选项：

| 选项 | 用途 |
| --- | --- |
| `--dry-run` | 预览目标、来源、预计文件变化，不执行初始化代码或应用配置 |
| `--verbose` | 展示完整安全审查信息 |
| `--yes` | 跳过普通确认，不跳过信任及审批校验 |
| `--force-profile` | 沿用现有“备份并替换生成 profile”行为 |

`--yes` 与 `--dry-run` 互斥。不接受软件位置参数、`--item`、`--preset`、`--shell` 或
`--proxy`。

保留 `--force-profile` 是为了让用户在独立刷新发生合并冲突时，也能独立选择备份替换，
无需重新选择软件。它不新增替换规则。

操作流程：

1. 按现有规则确定 OS、shell 和有效 preset。
2. 读取当前 Sys manifest，取得现有启用集合。
3. 展示整体 profile 刷新的 Plan。
4. 获得审批并重新验证输入后，生成和协调 profile。
5. 报告更新、无变化、需要处理或失败，并提示打开新 shell。

## 4. 文件、目标与加载顺序

继续使用：

```text
sys/<os>/shine.toml
sys/<os>/profile/base.pre.sh
sys/<os>/profile/base.post.sh
```

Windows 对应 `base.pre.ps1`、`base.post.ps1`。不新增文件、不增加 manifest 开关或选择
规则。仍需有效的 `sys/<os>/shine.toml`，但现有 v2 manifest 可以没有软件 items；这与
运行时 `sys-manifest.toml` 可以不存在是两个不同条件。

目标 shell 使用当前配置初始化得到的 shell 身份；不改变 `$SHELL` 解析及现有默认策略。
预览显示实际目标，避免将其描述成必然等于当前父进程。

继续支持现有组合：

- macOS：Zsh。
- Ubuntu、Arch：Bash/Zsh。
- Windows：PowerShell，沿用现有两个 profile 目标。

新入口在写入前确认解析出的 OS/shell 组合属于上述范围，且生成与加载目标一致。例如
macOS 的现有加载器固定写 `.zshrc`，不能用 Bash 身份生成集成后向该文件发布。拒绝不
支持的组合不改变现有 shell 检测政策，也不扩展支持范围。

继续写入现有 `~/.shine/profile/<os>-sys.{pre,post}.{sh,ps1}`，通过现有 Sys sentinel
区块加载，不直接 source 外部 preset 工作目录。

逻辑顺序保持为：

```text
启动文件中的 Sys pre 区块
  → 完整 base.pre
  → 已启用软件的 pre 集成

原有启动文件内容

启动文件中的 Sys post 区块
  → 完整 base.post
  → 已启用软件的 post 集成
```

软件集成仍按阶段、priority、manifest 顺序、声明顺序排序。base 文件内容不被拆解、
筛选或重新排序；沿用现有换行规范化和组合分隔。`base.post` 后仍可有软件 post 集成，
不承诺 base 中的设置一定最后生效。

沿用现有区块协调行为：首次接入时 pre 在开头、post 在末尾；合法已有区块按当前规则
保留或更新。Linux Bash/Zsh 切换仍会移除另一启动文件中的对应 Sys 区块，预览必须覆盖
相关文件。

不新增登录 shell 加载链、自定义启动路径或 shell 自动切换功能。

## 5. 已启用软件集成如何保留

独立刷新直接调用现有 `enabled_profile_items` 规则：

- 属于当前 OS；
- 不是 managed item；
- `profile_enabled = true`；
- 当前 preset 中仍存在该 item，且声明了 shell 集成。

组合器再按目标 shell 过滤具体集成。

`sys init` 不调用软件检测、版本探测、安装器或选择界面，不修改 `profile_enabled`、
执行结果、时间戳或软件回执，也不为自己创建虚构 item。

没有 `sys-manifest.toml` 时按空集合处理；有记录时只读。损坏或不支持的 manifest 仍按
现有规则阻止操作，不通过忽略状态继续写入。

这里保留的是**软件集成的启用意图**。如果当前 preset 修改或删除了某项集成，重新组合
仍采用当前定义；不会把旧生成代码永久冻结，也不会为此改写软件记录。

## 6. 与现有命令的关系

| 命令 | 职责 |
| --- | --- |
| `sys init` | 根据当前 base 和已有启用集合独立刷新 profile |
| `sys bootstrap` | 安装所选软件，按现有规则更新启用集合并刷新 profile |
| `sys profile enable/disable` | 改变指定软件集成状态并刷新 profile |

三者必须调用同一套生成逻辑，使用相同来源、排序、目标路径和合并规则。`sys init` 不
建立另一份独立配置，bootstrap 也不会把它覆盖成另一种格式。

复用范围是现有组合器、生成文件协调及加载区块逻辑；不合并三个入口各自的软件选择、
授权、回执写入和事务流程。新增入口不能调用 bootstrap 批处理并伪造一个成功软件
outcome，也不能借用 `profile enable` 来隐式启用某项。

软件启用集合由既有命令管理。`sys init` 不新增整体启用状态；关闭最后一个软件集成后，
base 内容仍然保留。

## 7. 重复执行、源更新与缺失文件

沿用现有协调语义：

| 情况 | 行为 |
| --- | --- |
| 首次执行 | 生成 profile，并接入现有加载区块 |
| 普通重跑且文件、基线、区块已收敛，无待清理产物 | 不重复区块，不重写等价文件，报告无变化 |
| 修改 base 后重新执行 | 按新模板与当前内容进行协调 |
| 修改 base，但未重新执行 | 已接入副本不变，新 shell 继续加载原副本 |
| 某个 base 文件缺失 | 该阶段 base 为空，仍组合该阶段已启用软件集成 |
| 两个 base 均缺失 | 只组合已有软件集成，不回退到其他来源 |
| base 和集成都为空 | 按现有流程协调为空的生成 profile 和加载区块，明确说明没有初始化内容 |
| 文件无法读取、来源捕获失败 | 报错，不把读取失败解释为不存在 |

不新增“所有 base 缺失就拒绝”的特殊政策。来源解析仍遵循现有有效快照规则，缺失处理
发生在快照解析之后，不额外从内置或其他 shell 来源补文件。

无变化规则不覆盖显式 `--force-profile`，也不阻止恢复缺失基线或清理已有 `.new`/`.merge`
产物；这些仍遵循原有协调行为，不为了统一结果标签修改它们。

删除原有 base 文件后，其内容会成为本次期望移除的部分，仍经过审批和现有三方合并，
不能直接覆盖用户修改。空文件可以表达同样的空模板意图。

这些规则来自当前 `read_base_profile` 的缺失即空处理及既有协调流程，不引入新的文件
生命周期模型。

## 8. 用户修改、冲突和结果

继续复用 active/base/new/merge 机制：

- 启动文件只协调 Shine 的托管区块，保留区块外的用户配置；不按模板替换整份启动文件。
- 用户修改生成的 active profile 后，按现有规则三方合并。
- 干净合并保留用户修改。
- 冲突时保留该阶段的 active，输出 `.new`/`.merge` 位置。
- 缺少合并基线时，沿用现有保护与人工处理规则。
- `--force-profile` 先备份再替换生成文件，不授予覆盖任意启动配置的权限。
- 继续保留现有 BOM、换行等价比较、文件权限及所有权检查。

三方合并保护适用于生成的 active profile，不适用于启动文件里的加载区块。后者由 Shine
生成，手改其中的加载语句可能在协调时被重建；自定义配置应放在区块外或 base 文件中。
真实更新沿用现有 Sys 的换行和边缘空行处理，不扩展为所有区块外字节绝对不变的承诺。

**不新增整次操作原子性。** 当前实现按阶段协调，某阶段需要处理时，另一阶段或加载
区块仍可能已更新。结果必须准确报告实际情况，不能宣称两个阶段都成功生效或全部未
改变。

`NeedsAction` 保留现有非致命退出惯例，但显示“需要处理”，不能只显示成功。

依据：[文件协调流程](../core/src/runtime/sys_profile/files.rs)、
[合并与保护](../core/src/runtime/sys_profile/reconcile.rs)。

## 9. 不可省的安全补充

新增入口不能直接裸调用内部写入函数，需要一个独立的 `sys-init` Plan 及 approved Core
入口。

Plan 绑定：

- OS、目标 shell、请求选项及完整有效 preset 快照；
- 当前软件启用集合及其来源 manifest；
- 实际 active/base/new/merge 文件；
- 所有可能修改的启动文件，包括 shell 切换涉及的另一文件；
- 备份等实际写入权限；
- 外部代码边界与信任状态。

执行沿用 Sys 操作锁，锁内重新验证审批；待恢复 journal 仍阻止新的变更。规划不执行
preset 代码，不探测软件，不请求管理员权限。

新入口须在首个 profile 写入前完成来源、manifest、目标组合、所有适用代码授权和生成
输入的预检。写入使用已审批快照中的字节，不在执行时改读可变的外部源。非事务交付只
说明写入中断后的恢复限制，不能成为边写边补授权或先发布 pre 再检查 post 信任的理由。

独立的 **base 代码授权检查不能删除**，理由是：

- 当前 `sys_profile_code_trusted` 只遍历启用 items，空集合会直接返回 `true`。
- 当前外部 profile requirement 由 `sys_external_code_requirements(..., item)` 派生。
- 新入口允许无 item 应用外部 base，直接复用这两个条件会缺少真实的代码授权对象。

依据：[当前信任检查](../core/src/runtime/sys_profile/mod.rs)、
[requirement 派生](../core/src/runtime/trust.rs)。

最小补充是为独立操作中的 base 交付建立固定、非软件项的整体 profile 信任身份，拟采用
`sys/profile/<os>`：

- 复用现有 `sys-profile-code` 能力和 `sys/<os>/` 完整类别快照身份。
- 不按 shell 拆分授权，不新增配置项或启停状态。
- 内置 base 使用发行来源信任；只有实际交付的 base 来源需要外部授权时才派生此
  requirement。base 在有效快照中不存在时，不凭空要求该目标的代码授权。
- 已启用软件集成继续保留其现有代码授权要求。
- 不凭一个软件项的授权自动授予整体 profile 身份。
- 新目标需接入现有信任解析、审查和授权流程；不能只增加一个显示标签。
- 在 `sys init` 中分别验证 base 与已有集成的授权，不能把“启用集合为空”或“整体 base
  已获授权”当作所有代码可信。复用写入函数不得撤去其他入口已有的授权检查。

当前 [CLI target 校验](../cli/src/trust.rs) 与 Core requirement 查询都不支持这个三段
Sys 目标，需要显式扩展；它与二段 `sys/<item>` 区分，OS 从目标解析并校验，不借用软件
item 查询。它是安全身份，不是新增用户配置项目。既有 item grants 保持原含义，不自动
迁移成整体授权；本次不要求 bootstrap/profile 入口改用新目标或统一其信任模型。

人工交互可以按现有机制给予本次精确代码交付的单次同意；`--yes` 仍需要匹配的已有
信任。作者声明和旧全局开关不替代授权。权限、所有权和快照校验不因代码信任而跳过。
不为 base 新增必填权限声明表：任意代码的作者声明仍是可选审查信息，Shine 自身的写入
权限由 Plan 派生；没有声明不等于代码可信，也不等于对任意启动代码进行了权限隔离。

审查将这些代码标记为“交付后续加载”。初始化期间不执行它们，但之后会由 shell 以
用户权限运行，不受 Shine 沙箱限制。撤销来源信任不会自动移除已交付副本或阻止 shell
继续加载它。

## 10. 不新增恢复状态

本设计采用**现有 bootstrap profile 的非事务交付边界**，不新建 profile-only journal，
不写初始化成功记录，也不伪造软件回执。

这是可复用的既有边界，而非取消现有保护：

- bootstrap 的 profile 组合本来就在 managed Sys recovery 范围之外。
- 现有 `sys profile enable/disable` 的区块事务及软件回执绑定继续保留，不改成非事务。
- 新 `sys init` 的 Plan 明确标识恢复限制，不声称 `sys recover` 可以回滚本次整体刷新。
- 中断后通过重新审查并执行 `sys init` 协调剩余内容；涉及用户修改时仍走合并或人工
  处理。

当前 journal 校验明确拒绝 previous/desired receipt 同时为空；既然不需要额外事务
保证，就没有必要为新命令改变该规则。

依据：[现有恢复边界](kb/architecture/data-flows.md#managed-sys-apply-and-recovery)、
[journal 回执要求](../core/src/runtime/sys_action_executor.rs)、
[ADR 0168](kb/decisions/0168-sys-profile-unchanged-receipt-boundary.md)。

## 11. 持续加载与当前终端

接入成功表示 profile 和启动加载入口已经更新，不表示当前终端的环境、函数或快捷键
已改变。

成功提示示例：

> Sys 初始化配置已更新。打开新的 shell 会话后加载；当前终端未自动重载。

不自动执行 `source`、dot-source、`exec shell` 或测试性加载，也不设置“执行成功后以后
跳过”的标记。

后续启动是否加载，遵循 shell 对既有启动文件的读取方式。用户主动跳过 profile、提前
退出启动脚本或使用其他启动路径时，Shine 不绕过这些行为。

## 12. 验收场景

| 场景 | 预期 |
| --- | --- |
| 无软件记录，只有 base | 能独立接入，不创建软件记录 |
| 缺失或无效的 preset manifest；运行 manifest 损坏 | 写入前失败，不按空集合掩盖错误 |
| 修改 base 中的变量、函数、vi 模式代码 | 完整交付，不拆成功能项 |
| Bash/Zsh 条件分支不同 | 保留代码，由实际 shell 执行对应分支 |
| 已启用多个软件集成 | 保留启用集合，不检测、不安装、不改回执 |
| 再次执行无变化 | 无重复区块，无无意义重写 |
| 修改 preset 后未执行 init | 已接入副本不变 |
| 单侧或全部 base 缺失 | 按空内容组合，与既有行为一致 |
| 用户修改 active | 保留修改或报告合并冲突 |
| 某阶段冲突 | 保留该阶段 active，准确报告其他已完成变化 |
| 外部 base 且启用集合为空 | 仍要求真实代码授权 |
| base 来自 overlay；base 内置但集成含外部代码 | 分别检查实际适用授权，不互相替代 |
| 内置 base 或有效快照中无 base | 不额外要求外部 base 授权，已有集成检查仍保留 |
| 对整体 profile 授予信任 | 不改变已有 item grants，不创建软件或初始化回执 |
| `--yes` 无匹配信任 | 拒绝交付 |
| 审查后源、manifest、目标文件变化 | 旧审批被拒绝 |
| 不支持的 OS/shell 组合或待恢复 journal | 写入前阻止操作 |
| bootstrap 与 init 交替执行 | 相同输入生成相同 profile，不相互丢失内容 |
| 调用被中断 | 不制造软件回执；可重新审查协调，明确非事务限制 |
| 正常执行或预览 | 不执行初始化代码，不自动重载当前终端 |
| BOM、CRLF、权限、shell 切换 | 保持既有保护和路径行为 |
| 加载区块手改与 active 手改 | 区分生成区块重建和 active 三方合并，不混淆保护范围 |

实现时使用隔离配置和临时启动文件验证；真实 shell 加载顺序测试另行执行，不能把本次
静态调研当作运行验证。PowerShell 原生验证与跨平台模拟结果分别记录。

## 13. 文档影响与实现边界

实现后，在中英文系统指南和命令参考中增加独立刷新流程，说明与 bootstrap 的区别、
更新时机和冲突处理；作者指南继续使用现有 base 文件布局。内部 KB 补充新入口及必要
信任边界，不把 PRD 或内部程序发布到公共手册。

当前仅保存设计，公共手册保持不变，避免把尚未实现的命令写成可用功能。实现新 Plan
操作与 trust target 时，按 KB 维护协议记录设计决定，检查安全契约及旧版本兼容性；
不能预先承诺无需 ADR 或 schema 调整。该检查不构成新增软件状态、迁移 preset 文件
布局或重建恢复系统的理由。
