# Shine Product Roadmap

> **Status:** strategic direction, not released behavior or an implementation specification.
> Each phase requires scoped issues and, when architecture or security semantics change, an
> accepted ADR. Current behavior remains defined by the code, tests, public manual, and existing
> ADRs.

## Goal

将 Shine 从面向开发者的 Preset 管理工具，逐步演进为：

> A trusted lifecycle runtime for portable personal capabilities.

短期先把 Preset、生命周期、安全模型和 Core API 做扎实；中长期再通过 GUI 和 AI 降低使用门槛。
每个 Phase 都应可独立发布，只有满足 exit criteria 后才进入下一阶段。

## Current Baseline

- Workspace 已包含 `shine-core` package，但绝大部分领域逻辑仍位于 `shine-cli` library。
- App、Shell、Sys 已有各自的 manifest/receipt、update/upgrade 和安全卸载能力，尚缺共同的
  structured lifecycle contract。
- `shine preset validate` 与 skill-first AI authoring 已存在，应继续作为 schema 和静态验证权威。
- External code 已有粗粒度 permission gates；GPG/age portable secrets 和 machine-local env 已存在。

## Guiding Principles

```text
AI
 ↓
Preset
 ↓
shine-core
 ↓
OS
```

- AI proposes; Shine validates and executes.
- Preset describes capabilities; Core owns lifecycle, security, ownership, and OS effects.
- CLI and UI are frontends, not independent lifecycle implementations.
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

现有 auto generator 在 read-oriented status/update 中执行的行为，需要独立 ADR 决定兼容迁移；最终
安全 Plan 不运行 generator、hook、artifact 或 bootstrap script。

## Phase 4 — Declarative Actions and Recovery

**Outcome:** 建立版本化 Declarative Action IR、permission derivation、operation journal、crash
recovery 和 per-target rollback，同时保留明确标记的 opaque code escape hatch。

**Exit criteria:**

- Fully declarative Preset 对相同输入产生稳定、确定的 Plan。
- 中断后可以安全 resume 或 rollback，且不会覆盖 operation 后产生的用户修改。
- 不可回滚或 opaque 的 action 在执行前明确显示。
- 所有内置 executable Preset 均已迁移或完成 execution、privilege、provenance 分类。

Rollback 不承诺跨 package manager、network 和多个 target 的全局事务；删除动作默认只能作用于
transaction-created 或 manifest-owned 资源。

## Phase 5 — Preset Developer Experience

**Outcome:** 在现有 `preset new` 和 `preset validate` 基础上，补齐 lint、authoring plan、test、pack、
fixtures、schema reference、examples 和 CI workflow。

**Exit criteria:**

- 作者可以在不激活或安装 Preset 的情况下完成 scaffold、validate、lint、plan 和 test。
- Machine-readable reports 有稳定 schema，适合 CI 和 AI repair loop。
- Pack 可复现，并拒绝 plaintext secret、private absolute paths、`node_modules` 和未声明代码。
- 实现这些命令时同步更新 English 与 Simplified Chinese manual。

## Phase 6 — `shine-ui`

**Outcome:** Tauri UI 直接复用 `shine-core`，覆盖浏览、已安装状态、details、permissions、plan、
install/update/uninstall，以及 env/secret management。

**Exit criteria:**

- UI 不 spawn CLI、不解析 stdout，也不复制 lifecycle logic。
- CLI 和 UI 对同一 snapshot 生成语义相同的 Plan 和 Apply result。
- UI 能恢复 journal 中的 operation state，且不持久化 secret plaintext。
- 至少一个 Tier-1 平台通过真实端到端 smoke test，其他平台能力有明确 matrix。

## Phase 7 — AI Preset Authoring

**Outcome:** 将现有 skill-first workflow 产品化为 draft、explain、validate、repair、permission
minimization 和 human review 闭环。

**Exit criteria:**

- AI 只能修改隔离 workspace 中的 Preset Draft，不能直接调用 OS mutation API。
- AI Draft 与手写 Preset 使用完全相同的 validate、plan、approval 和 apply。
- 模型只接触 secret handles，不接触 plaintext；安装前用户能查看完整 diff 和 permissions。
- Prompt injection、secret exfiltration、隐藏命令和权限遗漏有对抗测试。

## Phase 8 — Consumer Expansion

**Outcome:** 通过 UI 和 plain-language capability summaries 隐藏 TOML、shell、env syntax、平台路径
和加密细节，让普通用户可以安装、配置和移除个人 capabilities。

**Exit criteria:**

- 代表性 capability 无需终端或编辑 TOML 即可完成安装、配置和安全移除。
- 每次 mutation 前显示它做什么、访问什么、需要哪些 secret、将改变什么。
- Capability uninstall 不删除或反向修改其处理过的照片、Downloads、backup source 等用户内容。
- 不引入隐式后台执行、通用 workflow daemon 或 AI shell agent。

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
manager、full automation daemon 或 AI shell agent。

Shine 应专注：

> Package, deploy, secure, review, update and remove personal capabilities.

## Priority and Dependencies

```text
P0  Preset model, lifecycle, env/secrets, shine-core     Phase 1–2
P1  Plan, permissions, validation, trust                Phase 3
P2  Declarative actions, recovery, Preset DX            Phase 4–5
P3  shine-ui                                             Phase 6
P4  AI authoring                                        Phase 7
P5  Consumer UX, registry/sharing                       Phase 8–9
```

- Phase 2 依赖 Phase 1 的 structured lifecycle seam。
- Phase 3 必须在任何新的 AI/Registry execution path 之前完成。
- Phase 4 依赖 Phase 3 的 permission 和 plan model。
- Phase 6 可以提前做 read-only prototype，但 mutation release 必须使用 Phase 3 contract。
- Phase 9 只有在 bundle、permission history 和 signing verification 稳定后开始。

## Governance

- Roadmap 是方向与 phase gates，不是 live task list。
- 具体 deliverables、模块迁移映射和 acceptance tests 放入对应 planning issues 或 PRD。
- 改变现有设计或安全语义时，用 ADR 更新权威决策；Roadmap 不替代 ADR。
- 尚未发布的命令、schema 和 UI 不提前进入 public manual。
- User-visible release changes 必须同步 English 与 Simplified Chinese manual。
