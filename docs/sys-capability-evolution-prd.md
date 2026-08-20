# Shine sys 精确 Bootstrap 与 Preset 标准化规划

> **状态：已接受并实现，builtin preset 仍按 item 渐进迁移。** CLI、manifest、profile composition
> 和 permission contract 已由 [ADR 0027](kb/decisions/0027-sys-bootstrap-providers-and-profile-composition.md)
> 固化。已发布用户行为仍以 English / Simplified Chinese public manual 和命令 `--help` 为准。

## 1. 背景

`shine sys` 当前为 macOS、Ubuntu 和 Windows 提供 selectable bootstrap items，以及少量具有
desired state、receipt 和安全 remove 语义的 managed system resources。

当前 bootstrap preset 由两部分组成：

1. `shine.toml` 声明 items、named profiles 和少量 metadata；
2. 每个平台使用一个共享 `init.sh` / `init.ps1`，自行完成 item dispatch、软件检测、安装、更新
   检查，并通过 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE` 私有事件协议把结果返回 Rust。

Shell integration 也主要集中在平台级 `profile.pre.*` / `profile.post.*` 中。即使用户只选择一个
item，Shine 仍会安装包含多个工具检测与 activation 逻辑的整份平台 profile。

这个结构能快速添加 builtin 行为，但扩展一个普通 software item 时，作者必须同时理解：

- manifest item 与 named profile；
- 平台脚本的 dispatch 约定；
- status/update 私有事件协议；
- package manager、proxy、privilege 和 exit-code 差异；
- 平台级 shell profile 的顺序与条件逻辑；
- Rust 何时写 `sys-manifest.toml`、安装 profile 和停止后续 item。

对“检测一个命令，不存在时通过 Homebrew/APT/Winget 安装，再启用 shell integration”这类常见
需求，这个心智负担过高，也导致三个平台重复实现相似的生命周期代码。

## 2. 产品定位

`shine sys` 的目标是：

> 提供可靠、可审查的跨平台 bootstrap execution，并把常见 software item 的检测、安装和 shell
> integration 标准化；只对少量具有完整安全生命周期的系统资源提供持续管理。

责任边界为：

```text
Shine
  ├── 精确选择并执行 bootstrap items
  ├── 确保选中的 software 在本次 bootstrap 后可用
  ├── 组合和管理 Shine-owned shell integration
  ├── 记录 bootstrap outcome
  ├── 提供只读 update guidance
  └── 管理少量可安全撤销的系统资源

Homebrew / APT / Winget / mise / rustup / upstream installer
  └── 管理第三方软件版本、升级、依赖和软件仓库
```

Shine 标准化的是 bootstrap execution，不是 software management。

## 3. 非目标

- 不记录或持续验证 installation provenance。
- 不为 bootstrap software 增加 `shine sys upgrade <ITEM>`。
- 不执行第三方软件升级；`shine sys update [ITEM]` 保持只读。
- 不实现 package solver、dependency resolution、version constraint、version pinning 或 rollback。
- 不替代 Homebrew、APT、Winget、mise、rustup 等工具。
- 不猜测预先存在软件的安装来源。
- 不通过任意 shell-string install/update 字段开放未经约束的命令 DSL。
- 不把 common install steps 扩展成包含 download/extract/retry/fallback/rollback 的通用 workflow DSL。
- 不让 `sys/mise` 管理 `mise.toml`、plugins、backend selection 或 runtime versions。
- 不强制所有复杂安装迁移到 Rust；产品特定流程继续使用受约束的 per-item script escape hatch。
- 不让 named selection profile 同时承担 shell profile 内容或软件卸载语义。
- 不为了扩大 sys feature 数量而增加无法可靠 inspect/remove 的 managed driver。

## 4. 核心概念

### 4.1 Bootstrap item

`mode = "init"` 的 item 表示一次 ensure-present bootstrap：

1. 只读检测 software 是否已经可用；
2. 已可用时记录 `already-installed`，不升级；
3. 不可用时执行受约束的 package provider 或 per-item script；
4. 安装后重新检测；
5. 验证成功后记录 outcome，并启用该 item 声明的 shell integration。

Bootstrap outcome 是执行记录，不是实时版本状态，也不构成对第三方软件文件的 ownership。

### 4.2 Managed item

`mode = "managed"` 的 item 持续比较 desired/current state，并通过 receipt 支持幂等 apply 和安全
remove。它继续使用 `shine sys apply`、`shine sys uninstall` 和顶层
`shine upgrade sys/<ITEM>`，不进入 software bootstrap provider 模型。

当前 builtin managed item 是 `split-dns`；Rust core 还提供 `managed-file` driver，供自定义或未来
builtin preset 使用。managed driver 是严格评审后的例外，不是 software item 的默认实现方式。

### 4.3 两种 profile

本文严格区分：

- **Selection profile**：`recommended`、`minimal`、`all` 等 named item composition，决定一次
  bootstrap 选择哪些 items。
- **Shell integration profile**：登录 shell 中的 PATH、environment、activation、alias、function
  等配置，由 base profile 和 item-owned integrations 组合生成。

文档和代码命名应避免用无修饰的 `profile` 同时指代两者。

## 5. 精确 Bootstrap CLI

item 作为 positional arguments：

```text
shine sys bootstrap [ITEM]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy]
```

示例：

```bash
shine sys bootstrap mise
shine sys bootstrap rust mise
shine sys bootstrap --preset recommended
shine sys bootstrap
```

规则：

- positional items 与 `--preset` 互斥；
- 指定 items 时只接受当前 OS manifest 中 `mode = "init"` 的 item；
- managed item 返回明确错误，并提示 `shine sys apply <ITEM>`；
- 保留用户输入顺序；重复 item 在执行前去重，并保留首次出现的位置；
- 无 items 和 `--preset` 时保持现有 interactive/default behavior；
- 每个 item 独立检测与执行，首个失败仍停止后续 item；
- shell integration composition 每次 invocation 最多 finalize 一次；
- user-facing target/reporting 使用 canonical `sys/<item>`；
- 内部 manifest/receipt 继续使用 `(os_id, item_id)`，不把展示格式写入持久化 identity；
- CLI、未来 environment setup 和测试调用同一个 lower-level selection/execution API。

该能力只提高 bootstrap 粒度，不改变 software version ownership。

## 6. 标准 Software Item 模型

### 6.1 Manifest 结构

普通 package provider item 的模型：

```toml
[[items]]
id = "mise"
label = "mise"
description = "Install the mise development environment manager."
mode = "init"

[items.detect]
kind = "command"
command = "mise"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew"
package = "mise"
```

Windows 可以只替换 provider metadata：

```toml
[items.install]
kind = "package"
provider = "winget"
package = "jdx.mise"
```

这不是跨平台 universal manifest；每个平台仍拥有自己的 `presets/sys/<os>/shine.toml`，因此不需要
在单个 item 中引入复杂的 platform condition DSL。

### 6.2 Detection

初始版本只支持少量可静态校验的 detection kinds：

- `command`：命令是否可解析；可选结构化 `version_args` 仅用于显示；
- `path`：文件或目录是否存在；
- `any`：一组 command/path probes 中任一满足。

Detection 必须只读、无 shell 拼接、无隐式网络访问。安装前后的 detection 使用同一实现。无法用这些
原语准确检测的 item 使用 per-item script，而不是继续扩展 detection DSL。

### 6.3 Package provider

初始 provider 限定为 builtin preset 已有的重复路径：

- Homebrew formula；
- Homebrew cask；
- APT package；
- Winget package。

Rust 负责：

- 使用结构化 argv 构造固定 install action；
- package identifier validation；
- proxy 参数与环境变量；
- Unix privilege elevation 和 Windows elevation guidance；
- native process exit-code 检查；
- dry-run、安全输出和失败格式；
- 安装后 detection；
- 统一生成 `installed`、`already-installed`、`needs-action` 或 `failed` outcome。

Provider 只允许 ensure-present/install，不暴露 upgrade action，不解析或解决 dependencies。

### 6.4 Per-item script escape hatch

复杂 bootstrap 使用独立脚本：

```toml
[items.install]
kind = "script"
path = "install/yazi.sh"
```

目录约定：

```text
presets/sys/ubuntu/
  ├── shine.toml
  ├── install/
  │   ├── neovim.sh
  │   ├── yazi.sh
  │   └── astronvim.sh
  └── profile/
      ├── yazi.sh
      ├── fzf.sh
      └── zsh-vi-mode.zsh
```

新 per-item install script：

- 只实现该 item 的产品特定安装流程；
- 不负责 item dispatch；
- 不写 manifest 或 shell profile；
- 不实现 software upgrade；
- 不要求输出 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE`；
- 通过正常 stdout/stderr 和 exit code 返回结果；
- 由 Rust 在执行前后检测并决定最终 outcome。

如果安装成功后还需要人工动作，可以通过受校验的 manifest metadata 提供 `success_status` 和
`success_hint`，而不是让脚本发明新的 wire status。

## 7. Shell Integration 模型

### 7.1 Base profile

平台级 profile 只保留与单个 software 无关的公共逻辑，例如：

- `shine theme sync`；
- 通用 user-local PATH；
- Shine profile loader；
- 必要的平台 shell bootstrap。

建议文件名明确表达其职责：

```text
profile/base.pre.sh
profile/base.post.sh
profile/base.pre.ps1
profile/base.post.ps1
```

Homebrew、mise、Starship、Yazi、fzf 等 software-specific 内容应逐步移入 item-owned integration。

### 7.2 声明式 integration

常见 integration 使用有限的声明式原语。例如：

```toml
[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]
```

```toml
[[items.shell]]
shells = ["bash", "zsh"]
phase = "pre"
path = "$HOME/.local/bin"
```

```toml
[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "eza"
aliases = { ls = "eza --icons", ll = "eza -la --icons" }
```

初始原语限制为实际 builtin 需求已经证明必要的集合：

- `path`；
- `env`；
- guarded `eval`；
- guarded `source`；
- aliases。

值、argv、shell placeholder 和路径必须经过验证与 shell-specific escaping。不要增加任意
`command = "..."` shell string。

### 7.3 Item-owned fragment

Yazi 的 shell function、fzf 的平台 fallback、zsh-vi-mode 的多路径探测等复杂逻辑使用 fragment：

```toml
[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
fragment = "profile/yazi.sh"
```

Fragment 必须只包含该 item 的 shell integration，不应重新引入整个平台的 dispatch 或安装逻辑。

### 7.4 Composition 与顺序

Rust profile composer 使用稳定顺序生成 Shine-owned profile：

```text
base.pre
  → enabled item pre integrations
  → user profile content
  → enabled item post integrations
  → base.post
```

规则：

- `pre` / `post` 继续映射到现有 sys sentinel blocks，不能改写 sentinel ownership 边界；
- item 默认按 manifest 声明顺序排列，不按安装历史排列；
- 同 item 内按声明顺序排列；
- 只有确有顺序要求的少量内容允许可选 `priority`，相同 priority 仍按 manifest 顺序；
- `zsh-syntax-highlighting` 等必须靠后的内容通过明确 priority 表达；
- 生成内容必须保持 byte-deterministic 和 line-ending-aware reconciliation；
- PowerShell 继续同时维护 pwsh 与 Windows PowerShell profile，并保留 BOM；
- `$HOME` 下路径继续以 `$HOME/...` 形式写入。

### 7.5 Integration activation state

Shell integration 与软件卸载分开：

- bootstrap item 得到 `installed` 或 `already-installed` 后，启用其 integrations；
- bootstrap 失败时不启用；
- targeted bootstrap 只增加本次成功 item，不隐式禁用以前启用的 integrations；
- named selection profile 也不表示“删除 profile 外的 software/integration”；
- 顶层 `shine upgrade` 只根据当前 preset 重新渲染已启用 integrations，不升级 software；
- `sys-manifest.toml` 可以增加最小的 integration-enabled state，但不增加 provider/version
  provenance。

为避免“只能启用、不能退出”，提供命令：

```text
shine sys profile enable <ITEM>
shine sys profile disable <ITEM>
```

它们只管理 Shine-owned shell integration，不安装、升级或卸载 software。`enable` 必须先通过 item
detection；`disable` 只能移除 Shine 自己生成的 fragment/receipt，不触碰用户 profile 内容。

### 7.6 External preset 安全边界

Shell profile fragment 会在每次 shell 启动时执行，比一次性 install script 具有更长期的代码执行
能力。因此：

- `path`、静态 `env` 和 aliases 走严格声明式校验；
- `eval`、`source` 和 `fragment` 视为 persistent executable profile code；
- embedded builtin code 可以按内置信任边界使用；
- external preset/overlay 的 bootstrap/managed/update-check scripts 与 executable profile code
  必须通过全局 `allow_sys_code` 显式 opt in；项目配置不能授权自身；
- dry-run 必须列出将持久加载的 item、phase、shell 和 fragment path；
- update/status 的只读路径不得首次执行 external profile code；
- 配置开关为独立的 `allow_sys_code`；它同时覆盖 external/overlay 的 install-time sys script
  和 persistent executable profile code，不复用 app 专属的 `allow_app_hooks`。

## 8. Rust 与 Preset 的责任边界

### Rust core

- CLI parsing、selection 和 canonical target reporting；
- manifest schema validation；
- detection；
- package provider install argv；
- privilege、proxy、timeout、output limit 和 exit code；
- dry-run；
- outcome、manifest 与 enabled integration state；
- profile composition、排序、escaping、atomic write 和 reconciliation；
- managed resource desired state、receipt、apply/remove。

### Preset metadata

- item identity、label、description；
- detection metadata；
- package provider/package id 或 per-item script path；
- named selection profiles；
- shell integration declarations/fragments；
- required env、human guidance 和有限的 success hint。

### Per-item script

- 只处理无法用标准 provider 表达的产品特定 bootstrap；
- 不复制 Rust lifecycle；
- 不实现长期 software management。

## 9. 执行数据流

精确 bootstrap 的目标数据流：

1. 解析 positional items、named selection profile 或 interactive/default selection。
2. 校验所有 selected items 均为当前 OS 的 init items，并保持选择顺序。
3. 对每个 item 执行只读 detection。
4. 已存在则记录 `already-installed`；否则执行 package provider 或 per-item script。
5. 安装后重新 detection；未达到 declared present state 则失败。
6. 记录成功 outcomes，并更新 integration-enabled state。
7. Rust composer 一次性生成 base + enabled item integrations。
8. 通过现有 pre/post sentinel 和 profile reconciliation 安装 profile。
9. 输出 canonical `sys/<item>` 结果和一次 summary。

失败时必须保留 last-known-good generated profile；不得用部分生成内容替换现有 profile。

## 10. 分阶段迁移

### Phase 0：确认现有边界

当前已基本完成：

- public manual 已明确 sys 是 bootstrap + limited managed resources；
- `sys update` 是只读检查；
- `sys status` 展示 recorded outcome，不声称软件版本 current；
- managed resources 与 bootstrap software 已在命令和文档中区分。

剩余收尾是让 user-facing lifecycle reporting 一致展示 canonical `sys/<item>`，同时保持内部
`(os_id, item_id)` identity 不变。

### Phase 1：精确 Bootstrap Item（已完成）

- 实现 `shine sys bootstrap [ITEM]...`；
- 与 `--preset` 互斥；
- 复用现有 selection/execution API；
- 保持 legacy monolithic platform scripts；
- 为 canonical reporting 和 finalize-once 增加测试。

Phase 1 可以独立交付，直接解除 environment setup/composition 的主要粒度阻塞。

### Phase 2：标准 Detection 与 Package Provider（已完成）

- 接受最小 `detect` / `install` schema；
- 实现 Homebrew formula/cask、APT、Winget provider；
- Rust 统一 privilege、proxy、dry-run、exit-code 和 outcome；
- 新 script item 使用 per-item script contract，不再输出私有 status event；
- legacy `init.sh` / `init.ps1` 继续兼容。

### Phase 3：可组合 Shell Integration（已完成）

- 分离 base profile 与 item-owned integrations；
- 实现有限声明式原语和 fragment escape hatch；
- 引入 integration-enabled state；
- 实现稳定 composition、last-known-good 和 external-code permission；
- 评估并固化 `sys profile enable/disable` 命令。

### Phase 4：迁移 Builtin Presets（进行中）

建议顺序：

1. Windows Winget items；
2. macOS Homebrew formula/cask items；
3. Ubuntu APT items；
4. Ubuntu/macOS/Windows 的简单 activation；
5. Yazi、fzf、zsh-vi-mode、AstroNvim 等 per-item scripts/fragments。

每个平台迁移完成并有 golden tests 后，再弃用对应 monolithic dispatch/status protocol。兼容期内新旧
模型不能对同一 item 同时执行。

当前实现进度：Windows Winget items 已迁移；macOS 常见 Homebrew formula/cask items 已迁移；
Ubuntu 的直接 APT item 已开始迁移。三个平台的 shell profile 均已拆为 base + item-owned
integration。仍需产品特定 fallback 的 item 保留 legacy dispatcher/update-check 兼容路径；manifest
存在 `[items.install]` 时 Rust 一定选择标准执行器，因此不会对同一 item 双重执行。

## 11. 进入下一阶段的门槛

### Phase 1 → Phase 2

- 精确 item CLI 与 canonical reporting 已稳定；
- named/interactive/targeted selection 共用同一 API；
- targeted action 不改变未选择 item；
- profile finalize-once 已有测试。

### Phase 2 → Phase 3

- package provider argv、proxy、privilege 和 exit-code 在三平台有测试；
- standard item 与 legacy script 的互斥和 fallback 边界明确；
- failed detection/install 不写错误的成功 outcome；
- external install code permission 已有明确策略。

### Phase 3 → Phase 4

- generated profile 顺序与 bytes deterministic；
- pre/post sentinel、BOM、CRLF/LF 和 three-way merge invariants 有 golden coverage；
- item integration enable/disable 不触碰用户内容；
- external persistent profile code 必须显式授权；
- last-known-good profile 在 render/fragment failure 时保持不变。

## 12. 验收原则

1. 精确 bootstrap 只执行用户选择的 init items。
2. 重跑 bootstrap 只验证已存在 software，不把它包装成 upgrade。
3. `sys update` 始终只读；Shine 不新增 bootstrap software upgrade command。
4. package provider 只执行固定、结构化的 ensure-present argv。
5. 标准 item 作者无需理解 `SHINE_SYS_STATUS` / `SHINE_SYS_UPDATE` wire protocol。
6. 复杂安装和 profile 行为按 item 隔离，不回到平台级 giant dispatcher。
7. selection profile 只负责选择，不隐式卸载 software 或禁用其他 integrations。
8. profile integration enable/disable 只修改 Shine-owned content。
9. managed resource 与 bootstrap software 在 schema、命令、状态和文档中保持可区分。
10. targeted sys action 不改变未选择 item。
11. profile 生成失败保留 last-known-good，不能写入部分结果。
12. external persistent profile code 必须显式 opt in。
13. 所有新增 user-visible behavior 同步更新 English 和 Simplified Chinese manual。
14. 接受 CLI、manifest schema、profile composition 或 permission contract 后，用 ADR 记录最终设计；
    本文规划不替代实现事实源。

## 13. 推荐优先级

| 阶段 | 内容 | 优先级 |
| --- | --- | --- |
| Phase 0 | canonical reporting 收尾 | 已完成 |
| Phase 1 | `sys bootstrap [ITEM]...` | 已完成 |
| Phase 2 | 标准 detection/package provider/per-item script | 已完成 |
| Phase 3 | item-owned shell integration 与 profile composer | 已完成 |
| Phase 4 | builtin preset 渐进迁移 | 进行中 |
| 持续边界 | 新 reversible managed driver | 仅按真实需求评审 |
| 不进入路线 | provenance、software upgrade、package/runtime version management | 不做 |

这条路线只把重复、易错的 bootstrap lifecycle 和 profile composition 收进 Rust，不接管第三方软件
版本。普通扩展以 metadata 为主，复杂差异保留在小型 per-item script/fragment 中，从而同时降低 preset
作者心智负担并保持 Shine 的产品边界。
