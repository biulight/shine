# Clash Verge Rev 本地配置与远端订阅合并 PRD

## 1. 背景

用户希望像使用 Surge 一样，持续使用远端订阅提供的节点、规则和策略组，同时维护不会在订阅
刷新后丢失的本地配置，例如内网直连规则、固定的 LAN SOCKS 出口或个人策略组。

直接修改 Clash Verge Rev（以下简称 CVR）下载的订阅 YAML 不可行：订阅更新会覆盖该文件。CVR
提供 **Merge（扩展配置）/ Script** 两类 Profile Enhancement，能把本地的代理、策略组和规则与订阅
在运行前合成。本功能让 `shine` 管理一个**订阅级 Extend Config**，并**复用 Surge 已有的
LAN 规则服务器**，使本地规则可版本控制、可通过 presets overlay 个性化、可跨订阅长期保留，且对
远端订阅零侵入。

本 PRD 与仓库内的 **`surge` 预设逐项对齐**——实际 overlay 中两者共享同一套「规则真源 →
推送到 LAN 服务器 → 客户端按 URL 远程拉取并按 interval 自动刷新」的机制，只是把
Surge 的 `#!include` / `RULE-SET` 换成 mihomo 的 **Merge 配置 + `rule-providers`**。内置
惰性示例另外展示 HomeDir 本地文件、localhost HTTP 和远程 HTTPS 三种来源，但不改变
overlay 的远程共享方案。

## 2. 产品目标

- 新增 `clash-verge` app preset，安装一个组合增强源（`proxies` + `proxy-groups` +
  `rule-providers` + `prepend-rules`）。
- **规则改动零 CVR 交互**：规则真源与 Surge 共用同一份 `rules/`，改规则后只需
  `shine task run upload_surge`，mihomo 依 `rule-providers` 的 `interval` 自动刷新；
  `shine app artifact apply clash-verge` 可经 mihomo 外部控制器 API 立即应用（`surge-cli reload` 的等价物）。
- 本地规则明确以 **`prepend-rules`** 置于订阅规则之前，保证本地优先（mihomo 首匹配优先）。
- 不直接编辑或替换远端订阅 YAML，不保存订阅 URL、节点或订阅凭据。
- `shine` 管理的文件可由 presets overlay 覆盖，支持个人私有规则仓库随 `shine preset pull` / `shine upgrade` 更新。
- 与 CVR 已有的增强机制协作；不重造订阅下载、刷新或 mihomo 配置合成逻辑。

## 3. 非目标

- 不实现 Clash/mihomo 内核、订阅下载器或订阅格式转换。
- 不自动创建/登记 CVR 的增强槽：用户需**一次性**打开目标订阅的 Extend Config、Edit Rules、
  Edit Proxies 和 Edit Groups，让 CVR 创建四个绑定。
- **`shine` 只读 `profiles.yaml` 来解析当前订阅的 merge/rules/proxies/groups 绑定，只写这些绑定指向的
  用户内容文件，从不修改 `profiles.yaml`、绑定或订阅缓存**。绑定不全时绝不回退写全局文件。
- 不在 MVP 合并多个远端订阅为一个新的订阅。
- 不引入 `shine serve` / 本地监听端口；规则统一走 Surge 已有的远端 HTTPS 服务器。
- 不承诺将任意 YAML 数组做「智能合并」。规则、代理和代理组的顺序语义必须被保留。

## 4. 核心概念

### 4.1 远端订阅

由 CVR 自己下载和更新的 Profile。它是节点、远端 provider、默认规则和策略组的权威来源，`shine`
从不写入该文件。

### 4.2 本地 Merge 配置（`merge.yaml`）

`shine` 安装的**单个组合源 YAML**，由 `build.ts` 渲染到 CVR 的四类订阅增强文件：

| 片段 | 用途 | 顺序语义 |
| --- | --- | --- |
| `proxies` | 渲染到 Proxies editor 的 `prepend` | 前插且保留订阅代理 |
| `proxy-groups` | 渲染到 Groups editor 的 `prepend` | 前插且保留订阅组 |
| `rule-providers` | 留在订阅 Extend Config（merge） | 映射键合并，不替换订阅数组 |
| `prepend-rules` | 渲染到 Rules editor 的 `prepend` | 前插，本地优先 |

> **CVR 2.x 注意**：`prepend-rules` 等不再由 Extend Config 文件解释；真正的文件格式是独立的
> `{ prepend, append, delete }` editor 文档。上表键名是 shine 的组合源格式，由 build.ts 转换。

### 4.3 规则真源（与 Surge 共用 `rules/`）

规则列表（如 `lan.list` / `lan-socks.list` / `other-direct.list`）是纯 `DOMAIN-SUFFIX,x` /
`IP-CIDR,…,no-resolve` 的经典匹配行，**与 Surge 的 `RULE-SET` 列表完全同构**。它们由
`shine task run upload_surge` 推送到 LAN 服务器（如 `https://surge.biulight.internal/surge/rules/*.list`），
`merge.yaml` 的 `rule-providers` 以 `behavior: classical, format: text` 直接消费同一批文件——
一处维护、Surge 与 CVR 两端生效。

### 4.4 绑定

在 CVR 的目标订阅上分别打开 Extend Config、Edit Rules、Edit Proxies、Edit Groups，使其创建并绑定
四个增强项。`shine` 从 `profiles.yaml` 的 `current` → `option.<kind>` → item `file` 解析四个随机 UID
文件。**这是一次性动作**；改配置骨架后重新选择订阅一次，让 CVR 重新合成运行配置。

## 5. 用户流程

### 5.1 安装并配置

```bash
shine app install clash-verge
# 仅 build.ts 需要的控制器 env（LAN 出口/规则 URL 直接写在 overlay 的 merge.yaml 里）：
shine env set CLASH_CONTROLLER_URL   http://127.0.0.1:9097
shine env set CLASH_CONTROLLER_TOKEN <CVR 的外部控制器 secret>
```

安装把 `merge.yaml` 逐字写入 `~/.shine/clash-verge/merge.yaml`（`dest`，纯 Copy）。

**自动跑 build**：`clash-verge` 声明了 `post_install`/`post_upgrade` = `shine app artifact apply clash-verge`，
所以 `shine app install`/`shine upgrade` 在 `merge.yaml` 变化时会**自动**渲染并写当前订阅绑定的
四个增强文件。首次写入后会提示用户重新选择订阅，且不会对尚未进入运行态的 provider 发起必然 404。
首装时若绑定不全，会打印无害指引并跳过写入。**注意**：hook 只在本预设文件
（`merge.yaml`/骨架）变化时触发；高频的**改规则**走 `upload_surge`、不改 `merge.yaml`，故不触发 hook——
仍靠 provider `interval` 自动刷新或手动 `shine app artifact apply clash-verge`。

### 5.2 在 CVR 中一次性登记订阅增强 editors（无需手动粘贴）

在 CVR 的 Profiles 中，对目标订阅依次打开并保存空的 **Extend Config / Edit Rules / Edit Proxies /
Edit Groups**。不要使用底部的 Global Extend Config：其数组字段是覆盖语义，会删除订阅已有代理/组。

此后 build 只读 `profiles.yaml`，找到四个随机 UID 文件，将 overlay 组合源拆分写入。首次写入后重新
选择一次订阅；再次运行 build 即可立即刷新 providers。

> **归属与重载**：`clash-verge` 独占这四个绑定内容文件（整文件覆盖，带 managed 头），但不拥有绑定。
> CVR 不可靠监听外部写入，因此配置骨架变化后必须重新选择订阅才重新合成。

### 5.3 配置本地规则

规则真源与 Surge 共用（overlay 中的 `app/surge/rules/`）。编辑相应 `.list` 后：

```bash
shine task run upload_surge     # 推送规则到 LAN 服务器
shine app artifact apply clash-verge     # 经 mihomo API 立即刷新 provider（可选；否则按 interval 自动生效）
```

规则需引用 `merge.yaml` 中定义的策略组名称（`LAN Network` / `LAN PROXY` / `Other Direct`）；
内置模板不用虚构的策略组名启用任何规则。

### 5.4 刷新订阅

用户仍在 CVR 中执行「更新订阅」。CVR 重新下载远端 YAML，再合成本地 Merge 配置；本地文件与规则源
不受订阅刷新影响。用户可在 CVR 的最终配置/连接日志中验证 `prepend-rules` 顺序与策略组引用有效。

### 5.5 更新与卸载

```bash
shine upgrade                     # 仅重新渲染 shine 管理的 merge.yaml
shine app uninstall clash-verge   # 仅删 manifest 记录的文件；best-effort 运行 unbuild.ts 打印解绑指引
```

`uninstall` 仅删除 manifest 中记录的 `merge.yaml` 或恢复安装前备份；不删除 CVR 的订阅、节点、Profile
或增强绑定。`unbuild.ts` 提示用户在 CVR 中手动清理这些订阅增强内容。

## 6. 命令与预设设计

### 6.1 预设布局

```text
presets/app/clash-verge/                      # 本仓 base：仅示例
├── shine.toml
├── merge.yaml        # 唯一组合源：惰性注释示例（默认空、纯 Copy 安装、无模板）
├── build.ts          # bun：解析四个绑定、渲染 editor 文件、刷新 rule-providers
├── build.test.ts     # 组合源拆分、绑定解析、越界防护与幂等写入测试
└── unbuild.ts        # bun 脚本：打印在 CVR 解绑的指引（best-effort，卸载时非致命）
```

**base 仅示例，真实值在 overlay**（与 Surge `local-*.conf` 完全一致的分层）：本仓 `merge.yaml` 是
惰性、逐行注释、默认空的示例，并展示 HomeDir file / localhost HTTP / remote HTTPS 三种互斥
provider 写法；真实配置放在 presets overlay 的**同名文件** `app/clash-verge/merge.yaml`
（真实值**直接硬编码**、无模板），按同名路径覆盖 base。因不再用 `@@VAR@@`，overlay 的 `merge.yaml`
始终是合法 YAML（可直接编辑/校验/导入）；overlay 的 `shine.env.toml` 只留 `CLASH_CONTROLLER_URL/TOKEN`。

**不含 `rules/`**：实际 overlay 的规则真源与 Surge 共用，由 `upload_surge` 推送；
`merge.yaml` 的 `rule-providers` 引用同一 LAN 服务器。如果选择内置的 `type: file` 示例，用户必须
自行将 `.list` 文件放入 mihomo `HomeDir`；出于 mihomo 的路径安全限制，Shine 不会自动写入 CVR 私有目录或
配置 `SAFE_PATHS`。base build.ts 无私密、通用可用；overlay 无需自带 build.ts（`shine app artifact apply`
在 overlay 缺脚本时回退到 base 版）。

`shine.toml`：`dest = "~/.shine/clash-verge"`（shine 自管暂存区，不碰 CVR 私有存储）；`[artifact]`
声明 `script = "build.ts"` / `teardown = "unbuild.ts"` / `runtime = "bun"`；`merge.yaml` 纯 Copy
安装（无 `transforms`）。

> **为何用 bun 而非 bash（跨平台）**：app artifact 默认 `native` 直接执行脚本文件（依赖 shebang，
> 仅 Unix）；surge 是 macOS-only 故用 bash。CVR 跨平台（Win/mac/Linux），故 clash-verge 用
> `runtime = "bun"`——脚本经 `bun <script>` 运行，三平台通用（需 PATH 上有 bun，缺失则清晰报错）。
> 这也回应了「脚本逻辑落在本仓、与 surge 不一致」的观感：build.ts 是通用、无私密的刷新逻辑，可留
> base；真实值与凭据仍在 overlay（merge.yaml 硬编码 + shine.env.toml 的 `CLASH_CONTROLLER_*`）。

> **为何 reload 放在 artifact 而非 `post_install`/`post_upgrade`**：命令 hooks 只继承父进程环境、
> 拿不到 `[env]` 表；而 reload 需要 `CLASH_CONTROLLER_*`。只有 artifact 脚本会被注入 `[env]`（原样、
> 不解密），故刷新逻辑落在 `build.ts`。因此控制器令牌以**明文**键 `CLASH_CONTROLLER_TOKEN` 存储
> （`shine env set`）——**刻意不叫 `*_SECRET`**，因为加密的 `_SECRET` 键会以密文送达 build.ts。

### 6.2 env 契约

merge.yaml 无模板，故不消费任何 env；仅 `build.ts` 用以下键（由 build.ts 经 `[env]` 注入读取）：

| env | 用途 | 示例 |
| --- | --- | --- |
| `CLASH_CONTROLLER_URL` | mihomo 外部控制器（仅即时刷新用；不填则跳过刷新） | `http://127.0.0.1:9097` |
| `CLASH_CONTROLLER_TOKEN` | 控制器令牌（明文；无鉴权可留空；勿用 `_SECRET`/加密键） | （用户填） |
| `CLASH_PROFILES_FILE` | **可选**，覆盖 CVR `profiles.yaml` 索引路径 | 默认按平台探测 |

> `CLASH_CONTROLLER_*` 放共享 overlay 的 `shine.env.toml`（与 `SURGE_PROFILE` 并列）皆可；
> `CLASH_PROFILES_FILE` 是**每机**路径，只在自动探测不适用时设，且应放**每机的全局** `shine.env.toml`，
> **不要**放共享 overlay（否则 Windows 路径会破坏 macOS）。

### 6.3 `build.ts` 行为

`shine app artifact apply clash-verge` 依次做两件事：

1. **写四个订阅增强文件**：解析 overlay 胜出的组合源，再只读 `profiles.yaml` 的当前订阅
   merge/rules/proxies/groups 绑定。普通映射键（如 `rule-providers`）写 merge；proxies、proxy-groups、
   prepend-rules 分别转换为对应 editor 文件的 `prepend`。绑定不全或文件名越出 `profiles/` 时非致命
   跳过，绝不回退全局文件。内容变化时提示重新选择订阅并结束。
2. **刷新 rule-providers**：从最终生效的组合源 `rule-providers` 映射动态取得全部 provider 键，逐个向
   `CLASH_CONTROLLER_URL` 发 `PUT /providers/rules/<name>`，令 mihomo 立即重拉最新列表。名称作为单个 URL
   path segment 编码；不扫描远端订阅继承的 provider。`rule-providers` 缺失、为 null 或为空时明确跳过；
   若存在但不是映射则报告配置错误。`CLASH_CONTROLLER_URL` 未设也跳过刷新（规则仍按 interval 自动更新）；
   已设但不可达则以非零退出，由 `shine app artifact apply` 作为真错误上抛。
   只有第 1 步判断内容已是 current 时才刷新，避免 CVR 尚未重新合成导致 provider 404。

## 7. 配置语义与约束

- 组合源使用 `proxies` / `proxy-groups` / `prepend-rules`；build.ts 必须把它们拆进三个 editor
  文件的 `prepend`，绝不能把数组字段原样写进 CVR merge 文件。
- 规则顺序有语义：mihomo 从上到下首匹配；`prepend-rules` 保证本地规则先于订阅规则。
- `rule-providers` 用 `behavior: classical, format: text` 直接消费 Surge `.list`（含
  `IP-CIDR,…,no-resolve`）——两端共用一份规则源（详见 §8 尖刺 b 的逐行兼容验证）。
- `type: file` 的 `path` 必须位于 mihomo `HomeDir`；需访问其他目录时由用户显式配置
  `SAFE_PATHS`。localhost HTTP provider 需要在运行 mihomo 的同一设备上有独立服务。
- 内网规则服务器的 provider 必须显式设 `proxy: DIRECT`，避免 provider 更新沿订阅代理出站并以
  `EOF`/HTTP 503 失败。
- 若 provider 域名依赖系统 split DNS（例如 Windows NRPT），还必须镜像该域名后缀与 DNS 服务器：
  关闭 CVR DNS Override 时写组合源的 `dns.nameserver-policy`；保留 Override 时则在其 Advanced →
  Nameserver Policy 中写同一策略，并建议将该后缀加入 Fake IP Filter。mihomo 不读取 NRPT；CVR 的
  `dns_config.yaml` 在 Merge 之后应用，会整体替换 Merge 提供的 `dns` 映射。
- 本地 `proxy-groups` 若引用远端节点/组，名称变更会导致 mihomo 校验失败；错误应由 CVR/mihomo 清晰
  报告，`shine` 不在安装时假装验证远端订阅内容。
- 策略组启用 `include-all` 后，不要再在 `proxies` 显式列出同一个普通节点，否则 mihomo UI 会显示
  两个同名项。也不要用 `exclude-filter` 去重：CVR/mihomo 2.5.1 会把显式项一起过滤；让该节点只由
  `include-all` 收集一次即可。
- 本地规则可含私有网络信息；本仓 base 示例不含任何真实地址、域名、令牌或订阅 URL（真实值一律
  硬编码在 overlay 的 `merge.yaml`；控制器令牌走 overlay 的 `shine.env.toml`）。

## 8. 技术尖刺

1. **(a) Extend Config 接受度（Windows / CVR 2.5.1 已验证）**：Global Extend Config 的
   `proxy-groups` 整体覆盖订阅组，导致订阅规则引用的 `Proxies` 消失并校验回滚；全局
   prepend/append 也已移除。CVR 2.5.1 的正确入口是订阅级 Rules/Proxies/Groups 独立 editor 文件，
   格式均为 `{ prepend, append, delete }`。
2. **(b) 规则兼容**：Surge `.list` 作为 mihomo `behavior: classical, format: text` 的 provider
   payload 是否逐行兼容，尤其 `IP-CIDR,…,no-resolve`。
3. **(c) 控制器刷新**：mihomo 外部控制器 `PUT /providers/rules/<name>`（及可选的整体重载）的确切
   端点、鉴权头与成功码。
4. **(d)【可选/未来】远程 Merge**：CVR 是否支持**远程 URL 的 Merge 配置**——若支持，可把
   `merge.yaml` 也经 rsync 上传并让 CVR 绑定 URL，连一次性导入都自动化。

## 9. 验收标准（仅限 `shine` 可测项）

1. `shine app list`、`app info clash-verge` 和 `app install clash-verge --dry-run` 正确显示新预设、
   `dest = ~/.shine/clash-verge` 与 `merge.yaml`（纯 Copy，无 `[template]` 标记）。
2. overlay 的 `merge.yaml` 是合法 YAML（可 `yaml.safe_load`）；安装为**逐字 Copy**，无任何变量渲染。
3. 通过 presets overlay 覆盖 `merge.yaml` 后，`shine upgrade` 只更新该文件，其他文件与 CVR 订阅
   不被删除或改写。
4. `shine app uninstall clash-verge` 只处理 manifest 记录的文件；不删除 CVR 的 Profile、订阅或
   节点，并经 `unbuild.ts` 给出移除增强绑定的提示。
5. `shine app artifact apply clash-verge` 对缺少订阅级绑定给出非致命指引；内容首次写入后等待 CVR 应用；
   内容已应用而控制器不可达时以清晰错误非零退出。

> CVR 侧行为（规则刷新后仍前插于订阅规则、CVR 重启后仍生效）依赖 CVR 的合成实现，`shine` 无法
> 交付该保证，归入 §8 尖刺记录，不作为验收项。

## 10. 成功指标

- 用户能在不重新导入订阅的情况下，为一个或多个 CVR 订阅添加并长期保留本地规则。
- **改规则零 CVR 交互**：日常规则变更只需 `shine task run upload_surge`（+ 可选
  `shine app artifact apply clash-verge`），无需在 CVR 内操作。
- 订阅刷新后，本地规则丢失的反馈为零。
- 不耦合 CVR 私有存储格式，也不新增后台服务、本地监听端口或自动执行的外部 hook。
