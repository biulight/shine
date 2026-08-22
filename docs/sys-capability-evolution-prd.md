# Shine Sys Preset v2 与新设备初始化 PRD

> **状态：Sys Preset v2 已完成，Environment Setup 集成待后续实现。** 本文以 `v1.4.0` 为兼容
> 基线，取代此前“保留 monolithic platform dispatcher、渐进迁移 builtin item”的方案。Sys v2
> 核心已由 [ADR 0028](kb/decisions/0028-sys-preset-v2-single-execution-contract.md) 接受并在
> `v1.5.0` 实现发布，代码、English / Simplified Chinese public manual 和 changelog 已同步；第 9.2
> 节的 Environment Setup 集成仍是后续范围。

## 1. 摘要

`shine sys` 的核心职责是初始化新设备。自定义 sys preset 与 environment setup 是核心产品方向；
macOS、Ubuntu、Windows 可以拥有不同 item、不同安装方式和不同 shell integration，Shine 不追求把
平台能力抹平成 universal manifest。

Shine 统一的是每个 bootstrap item 的生命周期：

```text
选择 item
  → 只读检测
  → ensure-present 安装
  → 再次检测
  → 记录本次结果
  → 启用该 item 的 Shine-owned shell integration
```

从 v1.4 向 v1.5 开发期间，过渡实现曾同时保留 monolithic `init.sh` / `init.ps1` dispatcher，又
增加 Rust provider、manifest detection/install metadata 和 profile composer，形成双轨。v1.5.0
最终收敛到单一 v2 contract：所有 bootstrap item 显式声明 detection 与 provider/per-item script，
builtin preset 已一次性迁移，legacy bootstrap dispatcher 与私有 status wire protocol 已删除。

这是 public preset contract 的破坏性更新，最终随 Shine 1.5.0 发布。已安装用户的
`sys-manifest.toml` 运行状态继续兼容读取；破坏的是 preset authoring contract，不是用户已有状态。

## 2. 背景与问题

### 2.1 v1.4 模型

线上 v1.4 的一个 OS preset 由以下部分组成：

1. `shine.toml` 声明 item identity、selection profiles 与少量 managed metadata；
2. 一个共享 `init.sh` / `init.ps1` 负责 item dispatch、检测、安装和只读 update check；
3. 平台级 `profile.pre.*` / `profile.post.*` 包含多个软件的 activation；
4. 脚本通过 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE` 私有文本协议把结果返回 Rust。

该模型能快速添加 builtin 行为，但自定义 preset 作者必须理解平台 dispatch、状态协议、proxy、
privilege、结果格式和整个平台 profile。普通 ensure-present item 与复杂产品安装没有清晰边界。

### 2.2 过渡实现的问题

v1.5.0 收敛前的过渡实现已经具备精确 positional selection、Rust package provider、item-owned
profile composition 和 external-code permission，但 builtin 迁移尚不完整：

- macOS 与 Windows 的多数普通 package 已声明 provider，但旧安装函数仍在平台脚本中；
- Ubuntu 仍主要依赖 monolithic script；
- 旧脚本继续承载 bootstrap fallback 与 update checks；
- manifest、Rust 和脚本同时描述部分相同 lifecycle；
- 新架构的代码量已经产生，但旧架构的维护成本尚未消失。

长期保留双轨会让每个 bug 都需要判断它属于 provider、per-item script 还是 legacy fallback，也会
让 custom preset contract 存在两种有效写法。v1.5.0 因此完成了 v2 单一执行模型的收敛。

## 3. 产品定位

`shine sys` 定位为：

> 面向新设备初始化的、平台感知且可审查的 item execution framework；它为 environment setup 和
> custom presets 提供稳定编排接口，但不接管第三方软件的长期版本管理。

责任边界：

```text
Shine
  ├── 发现当前 OS preset
  ├── 精确选择并排序 bootstrap items
  ├── 执行 detection 与 ensure-present install
  ├── 统一 privilege、proxy、dry-run、timeout 和 outcome
  ├── 组合 Shine-owned shell integration
  ├── 保存 bootstrap run record 与 integration-enabled state
  └── 管理少量具有安全 remove 语义的 managed resources

平台 preset
  ├── 决定该平台有哪些 items
  ├── 选择固定 package provider 或平台专属 per-item script
  ├── 描述平台专属 detection
  └── 描述平台专属 shell integration

Homebrew / APT / Winget / rustup / mise / upstream installer
  └── 管理软件版本、仓库、依赖、升级与卸载策略
```

## 4. Goals

1. 为 environment setup 提供稳定的 `sys/<item>` target 与精确 bootstrap API。
2. 让 custom preset 作者无需实现 platform-wide dispatcher 或私有 status protocol。
3. 让普通 package item 只需 declarative metadata。
4. 让平台特有或复杂安装保留小型 per-item script escape hatch。
5. 让被选择且成功检测/安装的 item 才启用自己的 shell integration。
6. 删除 builtin 与 runtime 中的 legacy bootstrap fallback，消除双轨。
7. 保持 managed resources 与 bootstrap software 的命令、状态和安全语义分离。
8. 保持 v1.4 runtime state 可读取，避免升级后静默丢失已启用 profile。

## 5. Non-goals

- 不设计 universal cross-platform item manifest。
- 不要求不同 OS 拥有相同 item ID、provider、版本或安装步骤。
- 不实现 platform condition DSL；每个 OS 继续拥有独立 preset。
- 不实现 package solver、dependency resolution、version constraints、pinning 或 rollback。
- 不记录或推断 installation provenance。
- 不增加 bootstrap software upgrade/uninstall ownership。
- 不通过任意 shell-string metadata 创建 workflow DSL。
- 不把 download/extract/retry/fallback 等产品流程通用化到 Rust；这类逻辑留在 per-item script。
- 不让 selection profile 表示 software desired state 或隐式卸载。
- 不在本轮增加 `item_files` / include；先完成执行模型收敛，再用真实规模评估 manifest 分片。
- 不为了扩大 `sys` 功能而新增无法可靠 inspect/remove 的 managed driver。

## 6. 核心模型

### 6.1 Bootstrap item

`mode = "init"`（默认值）表示一次 ensure-present 初始化：

1. detection 已满足时记录 `already-installed`，不执行 install；
2. detection 未满足时执行 provider 或 per-item script；
3. install 成功后必须再次执行同一 detection；
4. 再检测仍失败时，item 失败且不得启用 integration；
5. 成功只形成 run record，不代表 Shine 拥有该软件。

v2 中每个 bootstrap item 必须同时声明 `detect` 与 `install`。不存在“缺少 install 时调用平台
`init.sh <item>`”的 fallback。

### 6.2 Managed item

`mode = "managed"` 表示 Shine 持续拥有 desired/current comparison 与安全 remove 语义的系统资源。
它继续使用：

```text
shine sys apply <ITEM>
shine sys uninstall <ITEM>
shine upgrade sys/<ITEM>
```

Managed item 不声明 bootstrap `detect`、`install` 或 shell integration。当前 `split-dns` 和已有
managed-file driver 保持该模型；新增 driver 仍需独立设计评审。

### 6.3 Selection profile 与 shell integration

- **Selection profile**：命名 item 列表，例如 `recommended`、`minimal`、`all`，只决定本次选择。
- **Shell integration**：item 对 Shine-owned shell profile 的贡献，拥有独立 activation state。

Selection profile 不是 replacement desired state。执行 `recommended` 不得禁用过去单独启用的 item。

## 7. Sys Preset v2 Contract

### 7.1 根 manifest

每个 `presets/sys/<os-id>/shine.toml` 必须显式声明：

```toml
version = 2
description = "Initialize this platform for development."
default_profile = "recommended"

[[items]]
id = "mise"
label = "mise"
description = "Install the mise development environment manager."
detect = { kind = "command", command = "mise", version_args = ["--version"] }
install = { kind = "package", provider = "homebrew", package = "mise" }

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]

[profiles.recommended]
items = ["mise"]
```

`version = 2` 本身启用 item-owned profile composition，不再需要
`profile_composition = true` feature flag。

### 7.2 Version validation

- 缺少 `version` 的 manifest 按 v1 识别并拒绝执行；
- `version = 1` 返回同一 migration error；
- 未知更高版本返回“当前 Shine 不支持该版本”，不得尝试降级解析；
- version error 必须在 detection、脚本执行、提权或 profile write 之前发生；
- `list` / `info` 可以展示 manifest incompatibility，但不得吞掉错误并借用 embedded category。

v1 错误应明确指出：删除 monolithic dispatcher、为每个 init item 增加 `detect` / `install`、迁移
software-specific profile，并链接对应版本的 migration guide。

### 7.3 Detection

v2 初始只支持经过 builtin 需求验证的只读 detection：

```toml
detect = { kind = "command", command = "mise", version_args = ["--version"] }
detect = { kind = "path", path = "$HOME/.config/nvim" }
```

复杂的任一匹配继续使用 `any`：

```toml
[items.detect]
kind = "any"
probes = [
  { kind = "command", command = "zerotier-cli" },
  { kind = "path", path = "/Applications/ZeroTier One.app" },
]
```

约束：

- detection 只读且不得隐式联网；
- command 使用结构化 argv，不经过 shell；
- path 允许 `$HOME` 形式，但必须经过现有安全展开与 validation；
- install 前后使用同一 detection；
- 无法可靠表达时，优先改善平台专属 detection primitive，而不是把检测藏进 install script；
- 不增加 arbitrary `command = "shell string"`。

### 7.4 Package provider

普通 ensure-present package 使用有限 provider：

- `homebrew`
- `homebrew-cask`
- `apt`
- `winget`

示例：

```toml
install = { kind = "package", provider = "winget", package = "jdx.mise" }
```

Rust 负责固定 argv、package identifier validation、proxy、privilege、timeout、bounded output、
exit code、dry-run、安装后 detection 与 outcome。Provider 只提供 install/ensure-present，不提供
upgrade、remove、version pin 或 dependency solver。

Provider 是否可用由当前 OS preset 决定，不要求同一 item 在其他 OS 使用相同 provider。Homebrew
本身、Rustup 或平台缺少适用 package 的 item 使用 per-item script。

### 7.5 Per-item install script

复杂安装使用平台专属文件：

```toml
[items.install]
kind = "script"
path = "install/neovim.sh"
```

推荐目录：

```text
presets/sys/ubuntu/
├── shine.toml
├── install/
│   ├── astronvim.sh
│   ├── homebrew.sh
│   ├── neovim.sh
│   └── yazi.sh
└── profile/
    ├── base.pre.sh
    ├── base.post.sh
    ├── fzf.sh
    ├── yazi.sh
    └── zsh-vi-mode.sh
```

Per-item script contract：

- 只安装一个 manifest item；
- 不做 item dispatch；
- 不写 `sys-manifest.toml` 或 shell profile；
- 不实现 update/upgrade；
- 不输出 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE`；
- 通过正常 stdout/stderr 与 exit code 返回；
- 从 `SHINE_SYS_PRESET_ROOT` 与 `SHINE_TARGET_HOME` 获取受支持路径；
- proxy/required env 由 Rust 按声明注入；
- 成功后仍由 Rust detection 决定最终 outcome。

Script path 必须是 preset root 下的安全相对路径：拒绝绝对路径、`..`、symlink escape 和不匹配当前
平台的扩展名。External/overlay script 受 `allow_sys_code` 控制。

### 7.6 Success guidance

安装成功但仍需人工操作时，可以声明：

```toml
[items.install]
kind = "package"
provider = "homebrew-cask"
package = "zerotier-one"
success_status = "needs-action"
success_hint = "open ZeroTier and join a network"
```

Hint 是受长度与控制字符校验的人类说明，不是 command string。

## 8. Shell Integration v2

### 8.1 Base 与 item ownership

`profile/base.pre.*` 和 `profile/base.post.*` 只允许平台公共内容，例如：

- Shine profile loader 所需环境；
- user-local PATH；
- terminal theme sync；
- 与单个 software 无关的平台初始化。

Homebrew、mise、Starship、Yazi、fzf、nvm 等内容必须归属对应 item。不得为了方便重新引入
platform-wide software block。

### 8.2 Declarative primitives

常见 integration 使用有限、可校验的结构：

- `path`
- `env`
- guarded structured `eval`
- guarded `source`
- `aliases`
- item-owned `fragment`

每个 `[[items.shell]]` 必须只选择其中一种内容形式。`eval` 是 argv，不是 shell string；`{shell}`
只能替换为当前受支持 shell。路径、变量名、alias 和 argv 均需严格 validation 与 shell-specific
escaping。

复杂 function 或平台探测放入 `profile/<item>.*` fragment。Fragment 只负责该 item 的 activation，
不得执行软件安装、修改 manifest 或分派其他 item。

### 8.3 Activation semantics

- bootstrap item 得到 `installed` 或 `already-installed` 后启用自己的 integrations；
- item 失败时 activation state 不变；
- targeted bootstrap 和 selection profile 都只增加成功 item，不禁用历史 item；
- `shine sys profile enable <ITEM>` 先运行 detection，成功后才启用；
- `shine sys profile disable <ITEM>` 只移除 Shine-owned generated content，不卸载软件；
- `shine upgrade` 可以依据当前 v2 preset 重渲染已启用 integration，但不得升级 bootstrap software。

### 8.4 Composition safety

排序固定为 phase、显式 priority、manifest item 顺序、同 item integration 顺序。输出必须 byte
deterministic；render/validation/fragment read 任一步失败时保留 last-known-good profile。

继续遵守现有 profile invariants：

- 只写 Shine sentinel block；
- `$HOME` 路径保持可移植表达；
- line-ending-only 差异不得触发改写；
- PowerShell 同步 pwsh 与 Windows PowerShell profile；
- 保留 PowerShell BOM；
- 不触碰 sentinel 外用户内容。

## 9. CLI 与 Environment Setup Contract

### 9.1 Bootstrap CLI

```text
shine sys bootstrap [ITEM]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy]
```

规则：

- positional items 与 `--preset` 互斥；
- 保留输入顺序，重复 item 按首次出现去重；
- 只接受当前 OS 的 init items；
- managed item 提示使用 `shine sys apply`；
- 无显式选择时保持 interactive/default-profile 行为；
- 首个 item 失败后停止后续执行；
- 一个 invocation 最多保存一次 manifest、compose 一次 profile；
- reporting identity 使用 `sys/<item>`，持久化 identity 保持 `(os_id, item_id)`。

### 9.2 Environment setup integration

Environment setup 必须调用与 CLI 相同的 lower-level selection/execution API，而不是拼接 shell command
或重新实现 provider 逻辑。输入是有序 canonical targets，输出是逐 item structured outcome 与最终
summary。

Environment definition 可以在不同 OS 选择不同 sys items，不要求名称或实现一致。缺失 item 是明确
validation error，不允许从其他 OS 借用 preset 或自动猜测替代项。

Targeted environment setup 不得改变未选择 item 的 software 或 activation state。

### 9.3 Status

`shine sys status` 继续展示 bootstrap run record。它不得把 `installed` / `already-installed` 描述成
实时 package 状态或版本 current，也不得为了显示状态执行 external code 或网络检查。

## 10. 移除 Bootstrap Software Update

`sys` 的核心是新设备初始化，不是长期 package manager。v2 删除 bootstrap software 的 live/read-only
update-check lifecycle：

- 删除 `shine sys update [ITEM]`；
- 删除 `SHINE_SYS_UPDATE` bootstrap protocol；
- 删除 platform script 中的 `check-update` dispatch；
- 顶层 `shine update` / `shine upgrade` 不检查或升级第三方 bootstrap software；
- `shine update` 仍可报告 managed system resources 的 desired-state change；
- `shine sys info` 可以展示 provider 与静态维护建议，但不得联网判断 package update。

用户通过 Homebrew、APT、Winget、mise、rustup 或上游工具管理第三方软件版本。English / Simplified
Chinese migration notes 必须列出该命令删除与替代方式。

## 11. External Preset Security

External preset 与 overlay 中以下内容视为 executable sys code：

- per-item install script；
- managed script；
- base profile；
- `eval`、`source`、fragment；

它们必须由 global-only `allow_sys_code = true` 授权；project config 不能授权自身。静态 detection、
provider metadata、PATH、env 和 aliases 不需要执行权限。

Preflight 必须在任何 detection side effect、installer、提权或 profile write 前完成，并一次性列出：

- 被阻止的 code kind 与路径；
- base preset 与 active overlay 来源；
- 实际生效的 global config 路径；
- 授权或保持阻止的明确操作。

Read-only `list`、`info`、`status` 与 completion 不得执行 external code。

## 12. Migration

### 12.1 Builtin migration

该迁移在开发分支中分 commit 完成，并在 v1.5.0 发布前满足以下门槛：

1. 三个平台所有 init item 都有 v2 detection 与 install；
2. 普通 Homebrew/APT/Winget item 使用 provider；
3. 复杂 item 移入 `install/<item>.*`；
4. software-specific profile 移入 metadata 或 `profile/<item>.*`；
5. 删除 `presets/sys/*/init.sh` 与 `init.ps1` monolithic bootstrap assets；
6. 删除 bootstrap legacy runner、fallback 与 status/update parser；
7. embedded preset tests 证明不存在未迁移 init item；
8. public manuals、migration guide、ADR、KB 与 changelog 同步。

### 12.2 External v1 presets

不提供隐式 runtime adapter。原因是 v1 monolithic script 无法安全、确定地推导出 per-item detection、
install ownership 或 shell integration。

Shine 2.0 对 v1 preset：

- 在 preflight 阶段拒绝；
- 不执行旧 script；
- 不安装或重写旧 profile；
- 输出 migration guide；
- `shine preset copy sys/<os>` 导出完整 v2 builtin 作为参考。

Migration guide 至少覆盖：version、detection、provider、per-item scripts、profile ownership、移除 update
check 和 external code permission。

### 12.3 Runtime state compatibility

`<shine_dir>/sys-manifest.toml` 是用户运行状态，不随 preset v1 一起废弃：

- 继续读取 v1.4 entries；
- 缺少 `profile_enabled` 的既有 init entry 默认视为 enabled；
- 只对当前 v2 manifest 中仍存在且声明 shell integration 的 item 参与 composition；
- 未知/已删除 item 不执行代码、不阻塞 bootstrap，也不立即破坏性删除 receipt；
- 正常的下一次成功 bootstrap/profile operation 再以当前 schema 原子保存；
- 不要求用户重新运行所有 installer 才能恢复 shell profile。

若最终实现需要不可隐式完成的状态清理，必须通过 `shine state migrate` 提供可预览、版本化迁移，
不能藏在普通 read command 中。

## 13. Failure Semantics

- Manifest/version/permission validation 失败：任何 item 都不得执行。
- Detection 失败：当前 item 失败，不执行 installer。
- Installer 非零退出或 timeout：当前 item 失败，不进行成功写入。
- Installer 返回零但 post-detection 失败：当前 item 失败并保留诊断。
- Item 失败：后续 item 停止，之前成功 outcome 可以记录，但 profile 只在完整 compose 成功后替换。
- Profile compose/write 失败：保留 last-known-good generated profile 与用户 sentinel 内容。
- Dry-run：不得执行 detection 之外的 installer、提权、state write 或 profile write；输出 provider/script、
  required env 与持久 integration 计划。
- Diagnostics 不得打印 secret value、proxy credential 或脚本接收的敏感环境变量。

## 14. Verification Plan

### 14.1 Manifest and custom preset

- v2 package、script、managed item 正常解析；
- 缺少/未知 version 有精确错误；
- init item 缺少 detection/install 被拒绝；
- managed item 混入 bootstrap fields 被拒绝；
- duplicate ID、非法 profile reference、非法 env key/path/package/argv 被拒绝；
- external full source 不借用 embedded category/file；
- overlay 继续按既有 per-file precedence；
- copy/export 包含 install/profile resources。

### 14.2 Execution

- Homebrew formula/cask、APT、Winget argv、proxy 与 privilege 均有纯 Rust 测试；
- already-present item 不执行 install；
- provider/script 完成后统一 post-detect；
- per-item script 只依赖 exit code，不解析 legacy status event；
- timeout/output limit 与错误 redaction 有覆盖；
- targeted selection 保序、去重且首错停止；
- environment setup 与 CLI 使用同一 execution entry point。

### 14.3 Profile

- 只启用成功 item；
- targeted/profile selection activation-additive；
- enable 先检测、disable 不卸载；
- priority/manifest/declaration order deterministic；
- fragment/base failure 保留 last-known-good；
- CRLF/LF、PowerShell BOM、双 profile 路径和 sentinel ownership 保持现有 golden behavior；
- v1.4 runtime entries 在 v2 下正确重组。

### 14.4 Removal gates

- embedded assets 中不存在 monolithic `init.sh` / `init.ps1`；
- production bootstrap path 中不存在缺失 install 的 legacy fallback；
- bootstrap 不再解析 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE`；
- CLI 不再暴露 `shine sys update`；
- 所有 builtin init item 通过 v2 validation；
- `rg`/characterization test 确认旧 dispatch functions 已删除而非仅不可达。

### 14.5 Repository checks

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --tests --benches -- -D warnings`
- `cargo nextest run --all-features`
- `cargo deny check bans licenses sources`
- `typos`
- website locale parity、typecheck 与 production build
- 代表性 macOS/Linux dry-run；Windows argv/profile 通过 CI 与 PowerShell tests
- `git diff --check`

## 15. Documentation and Decision Records

Sys v2 行为随 v1.5.0 落地时，已在同一 release change 中：

1. 新增 ADR，明确 supersede ADR 0027 中的 gradual legacy fallback 与 bootstrap update-check 部分；
2. 更新 sys architecture data flow 与 invariants；
3. 同步 `docs/kb/architecture/module-map.md` 的模块图与命令路由；
4. 更新 English 与 Simplified Chinese custom presets、system init、commands、built-in presets、
   configuration 和 troubleshooting；
5. 发布 v1 → v2 preset migration guide；
6. 在 changelog 明确列出 `shine sys update` 删除、manifest v2 和 runtime state compatibility；
7. 不把本 PRD、KB、ADR 或 release runbook 发布到 public manual。

本 PRD 是私有产品记录，不发布到 public manual；公开手册继续以已发布行为为准。

## 16. Release and Acceptance Criteria

Sys v2 的首个 stable release 是 Shine 1.5.0。以下条件中，1–3 和 5–12 是该版本的发布验收边界；
第 4 项 Environment Setup 集成仍是后续工作。整份 PRD 只有在全部条件满足后才视为完成：

1. Custom sys preset 只有一个 v2 bootstrap execution contract。
2. 三个平台 builtin preset 无 legacy bootstrap fallback。
3. 平台差异通过独立 OS manifest 与 per-item script 表达，不引入 universal platform DSL。
4. Environment setup 可稳定调用 ordered `sys/<item>` targets。
5. 普通 provider 与复杂 script 都遵循同一 pre/post detection 和 outcome 规则。
6. 选择未涉及的 item 不被安装、禁用或修改。
7. Profile failure 不破坏 last-known-good 与用户内容。
8. External executable sys code 必须 global opt-in，read-only commands 不执行代码。
9. v1 external preset 在执行前获得可操作 migration error。
10. v1.4 runtime state 可读取，升级不要求重装软件。
11. `shine sys update` 与旧 bootstrap protocols 已删除，文档给出 package-manager 替代路径。
12. English / Simplified Chinese manual、ADR、KB、changelog 与 implementation 一致。

## 17. Risks and Mitigations

| Risk | Mitigation |
| --- | --- |
| 外部 v1 preset 被破坏 | Major release、preview、明确 version error、migration guide、完整 v2 copy 示例 |
| 平台脚本拆分遗漏隐式共享状态 | 每个 item 声明 required env；公共逻辑仅进入 Rust 或 base；逐 item characterization tests |
| Windows/macOS/Linux 行为难以在单机验证 | 固定 argv 单测、平台 CI、preview smoke test，不以“跨平台相同”作为验收 |
| Profile 顺序变化 | 迁移前后 golden output 对比，显式 priority，保持 manifest order |
| Runtime state 与新 manifest 不一致 | ID-based tolerant load、未知 entry 不执行、必要清理走 versioned state migrate |
| Rust core 继续膨胀成通用 workflow engine | provider 保持有限；复杂流程必须回到 per-item script；新增 primitive 需 builtin 证据 |
| 为解决 manifest 长度过早增加 include | v2 首版维持单 `shine.toml`；完成执行迁移后再独立评估 authoring ergonomics |

## 18. Final Product Boundary

Sys v2 不承诺不同平台做相同的事。它承诺：无论某个平台选择 metadata provider 还是专属脚本，Shine
都能以同一套可审查生命周期精确初始化所选 item、记录结果并安全管理自身写入的 shell integration。

这使 custom presets 可以表达平台真实差异，为后续 environment setup 定义稳定 target contract，
也让 Shine 从 v1.4 的 platform-wide private script protocol 收敛为一个没有 legacy fallback 的新设备
初始化核心。
