# Shine Product Roadmap

> **Status:** strategic direction, not released behavior or an implementation specification.
> Each phase requires scoped issues and, when architecture or security semantics change, an
> accepted ADR. Current behavior remains defined by the code, tests, public manual, and existing
> ADRs.

## Goal

将 Shine 从面向开发者的 Preset 管理工具，逐步演进为：

> A trusted lifecycle runtime for portable personal capabilities.

短期先把 Preset、生命周期、安全模型和 Core API 做扎实；下一步先建立 CLI、AI adapter 和 UI
共同复用的 Frontend Service，再通过受限 AI 集成和可信人类界面降低使用门槛。每个 Phase 都应可
独立发布，只有满足 exit criteria 后才进入下一阶段。

## Current Baseline

- Phase 1、Phase 2 的 lifecycle contract 与 Core extraction 已完成；App、Shell、Sys 的领域执行、
  manifest/receipt 和 host ports 由 `shine-core` 持有，CLI 保留参数、交互和展示。
- Phase 3 已完成：所有 mutation 均使用 snapshot-bound security Plan；外部 App/Sys 代码使用绑定
  target、capability、code digest、trust layer 与 permission set 的 scoped trust grant。
- `shine preset validate` 与 skill-first AI authoring 已存在，应继续作为 schema 和静态验证权威。
- Read-oriented App status/update 默认不运行 generator；开发者可通过 `--run-generators` 显式执行并
  在内存中检查最终内容，写入仍只发生在显式 refresh 或已批准 mutation。GPG/age portable
  secrets 和 machine-local env 已存在。
- Phase 4 已收口：App 静态 Copy 与 key-owned JSON 的 create/update/relocate/remove，Shell 的
  launcher、cache、snapshot、rendered output 与 profile sentinel，以及 managed Sys file、split-DNS
  与显式 Sys profile sentinel 均已接入版本化 Action IR、domain operation journal、receipt/positive
  marker commit 与重新批准的显式 recovery Plan。`shine app recover`、`shine shell recover` 和
  `shine sys recover` 只在 destination、rollback、receipt 和 owned subset 的 fingerprint 仍匹配时
  回滚或清理；用户在中断后写入的其它 JSON key、profile 内容或文件状态会被保留并阻塞越界恢复。
  App hook/generator/artifact、已安装 Shell command body、Sys package/provider/bootstrap script、
  bootstrap profile composition 与 active/base/new/merge 三方合并文件已按 execution、privilege、
  provenance 和 rollback support 明确分类，并在执行前标记为 opaque 或不可事务恢复。Phase 4 不承诺
  package manager、network、跨 target 或命令处理过的用户数据的全局 rollback。
- Phase 5 已完成：`preset validate`、`lint`、`plan`、声明式合成 host-state fixture test、确定性
  unsigned bundle pack，以及由 shipped Rust types 与 live CLI help 生成的 schema reference 已形成
  authoring 闭环；App、Shell、Sys 示例均进入 CI。
- `2.0.0` 已在 macOS、Ubuntu 和 Windows 上完成真实 1.8 state 的 upgrade、lifecycle、uninstall
  与 recovery smoke test，并作为稳定版发布边界。后续 mutation frontend 仍须复用已经验证的
  lifecycle、安全与恢复 contract。
- Authoring 已有版本化 JSON reports，但真实 host 上的 inventory、inspection、Plan review、operation
  state 和 recovery 仍主要由 CLI 组装与呈现；Phase 6A 已建立版本化、脱敏的 Frontend Service
  inventory contract，并由 `shine list` 首先复用。Phase 6B 已建立脱敏 inspection 与复用 `PlanV1`
  的 review contract，CLI 状态检查与 Plan review 已接入。Operation state、events、recovery 与
  mutation conformance 仍待后续切片完成；`CoreRuntime` 继续是 workspace-internal。

## Guiding Principles

```text
AI clients ── Skill / MCP ──┐
CLI ────────────────────────┤
shine-ui ───────────────────┼── Frontend Service ── shine-core ── OS
Preset ─────────────────────┘
```

- AI proposes; Shine validates and plans; a human approves mutations; Core executes.
- Preset describes capabilities; Core owns lifecycle, security, ownership, and OS effects.
- CLI、MCP 和 UI 是 adapters/frontends，不是独立 lifecycle implementations。
- Shine 不成为 agent harness 或 AI shell agent；它向主流 harness 暴露受限、结构化、可审查的本地
  capability tools。
- 统一 identity、validation、plan、permissions 和 lifecycle result，同时保留 App、Shell、Sys
  各自的领域模型。
- Uninstall never touches user files or external Preset sources.

## Phase 1 — Stabilize Developer Lifecycle

**Outcome:** 稳定 Preset 模型、compatibility、manifest ownership、env/secrets，以及 App、Shell、Sys
共同的 structured lifecycle state 和 result。

**Exit criteria:**

- 所有内置 Preset 能对支持的平台分支完成静态验证。
- App、Shell、managed Sys 都有 install、update、upgrade、uninstall round-trip 测试。
- Targeted lifecycle 不修改其他 target；卸载只移除 Shine-owned state，并按策略恢复 backup 或移除
  managed keys。
- Schema、manifest、validation report 有明确版本和兼容策略，secret plaintext 不进入诊断或日志。

## Phase 2 — Extract and Expand `shine-core`

**Outcome:** 所有需要被 CLI/UI 复用的 product/domain logic 进入 `shine-core`；CLI 仅保留 argument
parsing、terminal interaction、formatting、prompts 和 distribution-specific behavior。

**Exit criteria:**

- 依赖方向只能是 `shine-cli → shine-core`；Core 不依赖 Clap、Tauri 或终端渲染库。
- App、Shell、Sys lifecycle 均通过 Core API 执行，CLI 不直接写对应 manifest 或底层资源。
- Fake/in-memory host 能在不访问真实 HOME、process、network 或管理员权限的情况下测试生命周期。
- 无 CLI 的 Rust harness 能完成 validate、inspect 和 plan；现有 CLI 行为通过兼容测试。

物理目录是否迁移到 `crates/shine-core` 和 `crates/shine-cli` 不是本阶段 gate；先完成依赖倒置，避免
同时扰动发布布局、`rust-embed` 路径和业务行为。

## Phase 3 — Security, Plan and Trust Model

**Outcome:** Preset 显式声明 filesystem、network、commands、administrator、env/secret 和 system
permissions；所有 mutation 都经过可审查、绑定输入 snapshot 的 Plan 和 approval。

**Exit criteria:**

- 每个 operation 都能计算 permission set；未声明或无法计算的权限 fail closed。
- Plan 不修改系统、不执行 Preset code，也不暴露 secret plaintext。
- Apply 只接受同一 source/state snapshot 的 Plan；权限扩大必须重新确认。
- 现有 coarse grants 有兼容迁移，升级不能静默扩大授权。

安全 Plan 不运行 generator、hook、artifact 或 bootstrap script；read-oriented status/update 仅在
显式 `--run-generators` 时执行 generator，且不得写入目标或 manifest。

## Phase 4 — Declarative Actions and Recovery (Complete)

**Outcome:** 建立版本化 Declarative Action IR、permission derivation、operation journal、crash
recovery 和 per-target rollback，同时保留明确标记的 opaque code escape hatch。

**Exit criteria:**

- Fully declarative Preset 对相同输入产生稳定、确定的 Plan。
- 中断后可以安全 resume 或 rollback，且不会覆盖 operation 后产生的用户修改。
- 不可回滚或 opaque 的 action 在执行前明确显示。
- 所有内置 executable Preset 均已迁移或完成 execution、privilege、provenance 分类。

Rollback 不承诺跨 package manager、network 和多个 target 的全局事务；删除动作默认只能作用于
transaction-created 或 manifest-owned 资源。

## Phase 5 — Preset Developer Experience (Complete)

**Outcome:** 在现有 `preset new` 和 `preset validate` 基础上，补齐 lint、authoring plan、test、pack、
fixtures、schema reference、examples 和 CI workflow。

具体命令边界、版本化报告、安全约束与交付顺序见
[`preset-developer-experience-prd.md`](preset-developer-experience-prd.md)。Authoring plan 是基于合成
状态的不可应用报告，不是 mutation approval；该边界由
[`ADR 0069`](kb/decisions/0069-hypothetical-preset-authoring-plans.md) 约束。

**Exit criteria:**

- 作者可以在不激活或安装 Preset 的情况下完成 scaffold、validate、lint、plan 和 test。
- Machine-readable reports 有稳定 schema，适合 CI 和 AI repair loop。
- Pack 可复现，并拒绝 plaintext secret、private absolute paths、`node_modules` 和未声明代码。
- 实现这些命令时同步更新 English 与 Simplified Chinese manual。

## Completed Release Gate — Stabilize Shine 2.0

**Outcome:** 在增加新的 mutation frontend 之前，证明 2.0 lifecycle、security 和 recovery contract
可以安全接管真实 1.8 state。

**Exit criteria:**

- macOS、Ubuntu 和 Windows 均完成从代表性 1.8 manifests、receipts、launchers、managed resources
  与 external Presets 出发的真实 upgrade smoke test。
- App、Shell、managed Sys 的 install/update/upgrade/uninstall 与显式 recovery 在支持平台完成真实
  lifecycle smoke；用户修改、legacy state 和中断 journal 场景保留安全边界。
- 发布 gate、失败证据与平台例外进入 release checklist；未通过的 mutation 路径不因 UI 或 AI
  adapter 而获得新的入口。

该 gate 已在 2.0.0 正式版前完成。后续真实 mutation surface 必须继续满足同等的跨平台验证和
安全边界，不能因新增 frontend 而降低要求。

## Phase 6 — Frontend Service and Conformance Contract

**Outcome:** 在 `shine-core` 之上建立 CLI、MCP 和 UI 共用的 frontend-neutral application service，
统一真实 host 上的 inventory、inspection、Plan review、operation state、recovery 和 lifecycle result，
同时保留 Core 的安全与领域边界。

实现边界与切片顺序见
[`frontend-service-conformance-prd.md`](frontend-service-conformance-prd.md)；contract redaction、
event projection 与 approval ownership 由 [`ADR 0077`](kb/decisions/0077-frontend-service-contract-and-approval-ownership.md)
约束。

**Exit criteria:**

- 定义版本化、可序列化的 inventory、inspection、operation-state、diagnostic 和 event contracts；
  复用现有 `PlanV1` 与 `LifecycleResultV1`，不把 raw errors、private paths、content、argv、environment
  values 或 secret plaintext 加入稳定 contract。
- CLI、MCP 和 UI 通过同一个 runtime bootstrap、immutable Preset snapshot、configuration capture 和
  Core lifecycle methods 工作；adapter 不实现 directory walker、manifest writes、permission
  derivation、approval matching 或 recovery decisions。
- Approval handoff 明确区分“请求 review”和“人类已审批”；任何 AI adapter 都不能代表用户生成
  approval、传递等价于 `--yes` 的 authority，或绕过 fresh Plan regeneration。
- CLI compatibility tests 与 frontend conformance harness 证明不同 adapter 对同一 captured snapshot
  产生语义相同的 Plan、operation state 和 result。
- 新稳定 contract、兼容策略、event redaction 和 approval ownership 先由 ADR 接受；底层
  `CoreRuntime` 可继续保持 workspace-internal，不因本阶段自动成为通用第三方 Rust API。

本阶段不交付完整 GUI、通用 remote API 或 agent runtime。Read-only prototype 可以先行，mutation
adapter 只有在 2.0 release gate 和本阶段 conformance gates 同时满足后才能发布。

## Phase 7 — Agent Integration and AI Preset Authoring

**Outcome:** 保留 skill-first workflow，并增加受限的 local MCP adapter，将现有 draft、explain、
validate、repair、permission minimization 和 human review 闭环接入主流 AI clients。Shine 作为这些
harness 的 capability runtime，而不是自己实现模型编排或 AI shell agent。

**Exit criteria:**

- 对拥有本地 shell 的 coding agents，`shine-preset-author` skill 继续是默认 workflow；Shine 不探测、
  修改或维护 Codex、Claude、Cursor、Gemini、Copilot 等客户端配置。
- MCP 第一阶段只暴露 bounded scaffold/schema/examples、validate、lint、authoring plan、fixture test、
  capability inventory/inspection 和 lifecycle Plan tools；不暴露 arbitrary command execution、secret
  decrypt、`--yes`、自动 source activation 或直接 apply。
- AI 只能修改隔离 workspace 中的 Preset Draft；AI Draft 与手写 Preset 使用完全相同的 validate、
  plan 和 permission contracts。
- 若未来允许 AI 发起 mutation，只能创建待审 review request；trusted CLI/UI 必须重新捕获 state、
  重新生成 exact Plan，并由人类明确批准后调用 Core。模型不能持有或复用 approval authority。
- 模型只接触 secret names/handles/opaque versions，不接触 plaintext；安装前用户能在可信 frontend
  查看完整 semantic diff、permissions、opaque effects 和 rollback classification。
- 同一 MCP tools 与 fixtures 至少在三个独立主流 clients 上通过 conformance/evaluation；Prompt
  injection、secret exfiltration、隐藏命令、权限遗漏和诱导自动批准有对抗测试。

## Phase 8 — `shine-ui` and Consumer Expansion

**Outcome:** 先将 `shine-ui` 建成可信的人类 control surface，覆盖 Plan/permission review、explicit
approval、operation progress、recovery 和 secret input；随后再用 plain-language capability summaries
隐藏 TOML、shell、env syntax、平台路径和加密细节，让普通用户安全使用 personal capabilities。

**Exit criteria:**

- Tauri UI 直接复用 Phase 6 Frontend Service；不 spawn CLI、不解析 stdout，也不复制 lifecycle、
  approval、manifest 或 recovery logic。
- 第一个可发布 slice 覆盖浏览、已安装状态、details、permissions、Plan approval、operation state、
  recovery 和 env/secret input；不以 Preset editor、AI chat wrapper 或 registry client 作为前置条件。
- CLI 和 UI 对同一 snapshot 生成语义相同的 Plan 和 Apply result；UI 能恢复 journal 中的 operation
  state，且不持久化 secret plaintext 或向 AI adapter 暴露 secret input。
- 至少一个 Tier-1 平台通过真实端到端 smoke test，其他平台能力有明确 matrix。
- 代表性 capability 无需终端或编辑 TOML 即可完成安装、配置和安全移除；每次 mutation 前显示它做
  什么、访问什么、需要哪些 secret、将改变什么，以及哪些 effect 不可事务恢复。
- Capability uninstall 不删除或反向修改其处理过的照片、Downloads、backup source 等用户内容；
  不引入隐式后台执行、通用 workflow daemon 或 AI shell agent。

若未来让 Core 直接执行 user-data workflows，必须先用独立 ADR 定义 run transaction、preview、
backup/undo 和 ownership boundary；它不是 Preset uninstall/rollback 的自然延伸。

## Phase 9 — Sharing and Registry

**Outcome:** 在本地安全模型和 bundle format 稳定后，建立签名、author identity、provenance、
compatibility、permission history、revocation 和 verified distribution。

**Exit criteria:**

- Bundle 在解包或执行前验证 content hash 和 signature。
- 更新不能绕过 pinned identity、revoked key 或 permission-delta approval。
- Registry 不接受 plaintext secret、private machine paths 或未声明 executable code。
- Compromised key、恶意更新、rollback 和 registry unavailable 场景完成演练。

## Product Boundary

Shine 不应演进成 package version manager、Homebrew/Nix 替代品、generic workflow SaaS、shell plugin
manager、full automation daemon、agent harness 或 AI shell agent。

Shine 应专注：

> Package, deploy, secure, review, update and remove personal capabilities.

## Priority and Dependencies

```text
P0  Preset model, lifecycle, env/secrets, shine-core     Phase 1–2
P1  Plan, permissions, validation, trust                Phase 3
P2  Declarative actions, recovery, Preset DX            Phase 4–5
DONE Shine 2.0 real-state stabilization                 Completed release gate
NOW Frontend Service and conformance contract           Phase 6 (6A–6B complete; 6C next)
P4  Agent Skill + local MCP integration                 Phase 7
P5  Trusted shine-ui and consumer UX                    Phase 8
P6  Registry and sharing                                Phase 9
```

- Phase 2 依赖 Phase 1 的 structured lifecycle seam。
- Phase 3 必须在任何新的 AI/Registry execution path 之前完成。
- Phase 4 依赖 Phase 3 的 permission 和 plan model。
- Phase 6 是真实 lifecycle MCP tools 与 `shine-ui` 的共同前置；read-only contract/prototype 可与 2.0
  稳定化并行，但 mutation adapter 必须同时通过 release gate 与 conformance gate。
- Phase 7 的 authoring-only MCP 可先复用 Phase 5 reports；访问真实 host state 或发起 review request
  的 tools 依赖 Phase 6。
- Phase 8 可以提前做 UX prototype，但可发布 UI 必须复用 Phase 6 service；不得以 Tauri adapter
  反向定义 Core contract。
- Phase 9 只有在 bundle、permission history 和 signing verification 稳定后开始。

## Governance

- Roadmap 是方向与 phase gates，不是 live task list。
- 具体 deliverables、模块迁移映射和 acceptance tests 放入对应 planning issues 或 PRD。
- 改变现有设计或安全语义时，用 ADR 更新权威决策；Roadmap 不替代 ADR。
- Phase 6 implementation 必须先接受 Frontend Service contract ADR；Phase 7/8 adapters 只能扩展该
  contract，不能为某个 AI client 或 UI 复制 lifecycle semantics。
- 尚未发布的命令、schema 和 UI 不提前进入 public manual。
- User-visible release changes 必须同步 English 与 Simplified Chinese manual。
