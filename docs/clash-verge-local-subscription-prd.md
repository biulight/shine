# Clash Verge Rev 本地配置与远端订阅合并 PRD

## 1. 背景

用户希望像使用 Surge 一样，持续使用远端订阅提供的节点、规则和策略组，同时维护不会在订阅
刷新后丢失的本地配置，例如内网直连规则、固定的 LAN SOCKS 出口或个人策略组。

直接修改 Clash Verge Rev（以下简称 CVR）下载的订阅 YAML 不可行：订阅更新会覆盖该文件。CVR
提供 **Merge（扩展配置）/ Script** 两类 Profile Enhancement，能把本地的代理、策略组和规则与订阅
在运行前合成。本功能让 `shine` 管理一个 **env 渲染的 Merge 配置**，并**复用 Surge 已有的
LAN 规则服务器**，使本地规则可版本控制、可通过 presets overlay 个性化、可跨订阅长期保留，且对
远端订阅零侵入。

本 PRD 与仓库内的 **`surge` 预设逐项对齐**——两者共享同一套「规则真源 → 推送到 LAN 服务器 →
客户端按 URL 远程拉取并按 interval 自动刷新」的机制，只是把 Surge 的 `#!include` / `RULE-SET`
换成 mihomo 的 **Merge 配置 + `rule-providers`**。

## 2. 产品目标

- 新增 `clash-verge` app preset，安装一个 env 渲染的 Merge 配置（`proxies` + `proxy-groups` +
  `rule-providers` + `prepend-rules`）。
- **规则改动零 CVR 交互**：规则真源与 Surge 共用同一份 `rules/`，改规则后只需
  `shine task run upload_surge`，mihomo 依 `rule-providers` 的 `interval` 自动刷新；
  `shine app build clash-verge` 可经 mihomo 外部控制器 API 立即应用（`surge-cli reload` 的等价物）。
- 本地规则明确以 **`prepend-rules`** 置于订阅规则之前，保证本地优先（mihomo 首匹配优先）。
- 不直接编辑或替换远端订阅 YAML，不保存订阅 URL、节点或订阅凭据。
- `shine` 管理的文件可由 presets overlay 覆盖，支持个人私有规则仓库随 `shine pull` / `shine upgrade` 更新。
- 与 CVR 已有的增强机制协作；不重造订阅下载、刷新或 mihomo 配置合成逻辑。

## 3. 非目标

- 不实现 Clash/mihomo 内核、订阅下载器或订阅格式转换。
- 不自动猜测用户要绑定的订阅；Merge 配置绑定到哪个 CVR Profile 必须由用户在 CVR 内显式完成。
- **不代替用户在 CVR 内完成 Merge 配置的一次性导入/绑定**——该绑定由 CVR 在其私有存储中拥有，
  `shine` 从不写 CVR 的 `profiles.yaml`、增强绑定或订阅缓存（这些是应用内部状态，格式和存储位置
  可能随 CVR 版本变化）。这是本方案唯一无法自动化的手工步，等价于 Surge 需要一次性布线 `#!include`。
- 不在 MVP 合并多个远端订阅为一个新的订阅。
- 不引入 `shine serve` / 本地监听端口；规则统一走 Surge 已有的远端 HTTPS 服务器。
- 不承诺将任意 YAML 数组做「智能合并」。规则、代理和代理组的顺序语义必须被保留。

## 4. 核心概念

### 4.1 远端订阅

由 CVR 自己下载和更新的 Profile。它是节点、远端 provider、默认规则和策略组的权威来源，`shine`
从不写入该文件。

### 4.2 本地 Merge 配置（`merge.yaml`）

`shine` 安装的**单个** YAML，作为 CVR 的 Merge（扩展配置）增强，内容分四部分：

| 片段 | 用途 | 顺序语义 |
| --- | --- | --- |
| `proxies` | LAN SOCKS/HTTP 等本地代理节点 | 与订阅代理并列 |
| `proxy-groups` | 个人策略组（`type: select`, `include-all: true`） | 与订阅组并列，`include-all` 保持与订阅节点同步 |
| `rule-providers` | 远程规则源（`type: http`, `behavior: classical`, `format: text`, `interval`） | 指向 Surge 同一 LAN 服务器 |
| `prepend-rules` | 引用上述 provider 的规则，前插于订阅规则之前 | **prepend**，本地优先 |

> **CVR 键名注意**：使用 CVR/mihomo 的真实键 `prepend-rules` / `append-rules` /
> `prepend-proxies` 等，**不要**用泛化的 `prepend:` 列表。

### 4.3 规则真源（与 Surge 共用 `rules/`）

规则列表（如 `lan.list` / `lan-socks.list` / `other-direct.list`）是纯 `DOMAIN-SUFFIX,x` /
`IP-CIDR,…,no-resolve` 的经典匹配行，**与 Surge 的 `RULE-SET` 列表完全同构**。它们由
`shine task run upload_surge` 推送到 LAN 服务器（如 `https://surge.biulight.internal/surge/rules/*.list`），
`merge.yaml` 的 `rule-providers` 以 `behavior: classical, format: text` 直接消费同一批文件——
一处维护、Surge 与 CVR 两端生效。

### 4.4 绑定

在 CVR 的「Profiles → 订阅 → 增强链」里，将 `merge.yaml` 作为一个 Merge 配置加入并保存。绑定关系
由 CVR 维护，`shine` 不通过修改内部数据库或配置文件创建。**这是一次性动作**：绑定后，改规则只需
`upload_surge` + 自动/手动刷新，改 `merge.yaml` 的骨架（proxies/组/provider 定义）时才需重新导入。

## 5. 用户流程

### 5.1 安装并配置

```bash
shine app install clash-verge
# 仅 build.ts 需要的控制器 env（LAN 出口/规则 URL 直接写在 overlay 的 merge.yaml 里）：
shine env set CLASH_CONTROLLER_URL   http://127.0.0.1:9097
shine env set CLASH_CONTROLLER_TOKEN <CVR 的外部控制器 secret>
```

安装把 `merge.yaml` 逐字写入 `~/.shine/clash-verge/merge.yaml`（`dest`，纯 Copy）。CLI 打印该
绝对路径与在 CVR 中导入为 Merge 配置的步骤。

### 5.2 在 CVR 中一次性绑定

打开 Clash Verge Rev → Profiles → 目标订阅 → 增强链 → 添加/导入 `~/.shine/clash-verge/merge.yaml`
为 Merge 配置并保存。此后本地代理、策略组、规则会在订阅每次合成时叠加，且规则内容按 provider
`interval` 自动刷新。

### 5.3 配置本地规则

规则真源与 Surge 共用（overlay 中的 `app/surge/rules/`）。编辑相应 `.list` 后：

```bash
shine task run upload_surge     # 推送规则到 LAN 服务器
shine app build clash-verge     # 经 mihomo API 立即刷新 provider（可选；否则按 interval 自动生效）
```

规则需引用 `merge.yaml` 中定义的策略组名称（`Local Network` / `LAN SOCKS Rules` / `Other Direct`）；
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
或增强绑定。`unbuild.ts` 提示用户在 CVR 增强链中手动移除该 Merge 配置，避免残留失效引用。

## 6. 命令与预设设计

### 6.1 预设布局

```text
presets/app/clash-verge/                      # 本仓 base：仅示例
├── shine.toml
├── merge.yaml        # 唯一 payload：CVR Merge 配置的**惰性注释示例**（默认空、纯 Copy 安装、无模板）
├── build.ts          # bun 脚本（runtime="bun"，跨平台）：经 mihomo 外部控制器 API 刷新 rule-providers
└── unbuild.ts        # bun 脚本：打印在 CVR 解绑的指引（best-effort，卸载时非致命）
```

**base 仅示例，真实值在 overlay**（与 Surge `local-*.conf` 完全一致的分层）：本仓 `merge.yaml` 是
惰性、逐行注释、默认空的示例；真实配置放在 presets overlay 的**同名文件** `app/clash-verge/merge.yaml`
（真实值**直接硬编码**、无模板），按同名路径覆盖 base。因不再用 `@@VAR@@`，overlay 的 `merge.yaml`
始终是合法 YAML（可直接编辑/校验/导入）；overlay 的 `shine.env.toml` 只留 `CLASH_CONTROLLER_URL/TOKEN`。

**不含 `rules/`**：规则真源与 Surge 共用，由 `upload_surge` 推送；`merge.yaml` 的 `rule-providers`
引用同一 LAN 服务器。base build.ts 无私密、通用可用；overlay 无需自带 build.ts（`shine app build`
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

merge.yaml 无模板，故不消费任何 env；仅 `build.ts` 需要以下两个键（放 overlay 的 `shine.env.toml`，
与 `SURGE_PROFILE` 等并列，由 build.ts 经 `[env]` 注入读取）：

| env | 用途 | 示例 |
| --- | --- | --- |
| `CLASH_CONTROLLER_URL` | mihomo 外部控制器 | `http://127.0.0.1:9097` |
| `CLASH_CONTROLLER_TOKEN` | 控制器令牌（明文，供 `build.ts`；无鉴权可留空；勿用 `_SECRET`/加密键） | （用户填） |

### 6.3 `build.ts` 行为

`shine app build clash-verge` 对 `merge.yaml` 中每个 `rule-providers` 键
（`lan` / `lan-socks` / `other-direct`）向 `CLASH_CONTROLLER_URL` 发
`PUT /providers/rules/<name>`，令 mihomo 立即重新拉取 LAN 服务器上的最新列表。幂等、非破坏，
从不写 CVR 配置或私有存储；任一调用失败以非零退出，由 `shine app build` 作为真错误上抛。

## 7. 配置语义与约束

- 规则数组用 **`prepend-rules`** 表达本地优先，**不要**依赖泛化 YAML merge 追加数组。
- 规则顺序有语义：mihomo 从上到下首匹配；`prepend-rules` 保证本地规则先于订阅规则。
- `rule-providers` 用 `behavior: classical, format: text` 直接消费 Surge `.list`（含
  `IP-CIDR,…,no-resolve`）——两端共用一份规则源（详见 §8 尖刺 b 的逐行兼容验证）。
- 本地 `proxy-groups` 若引用远端节点/组，名称变更会导致 mihomo 校验失败；错误应由 CVR/mihomo 清晰
  报告，`shine` 不在安装时假装验证远端订阅内容。
- 本地规则可含私有网络信息；本仓 base 示例不含任何真实地址、域名、令牌或订阅 URL（真实值一律
  硬编码在 overlay 的 `merge.yaml`；控制器令牌走 overlay 的 `shine.env.toml`）。

## 8. 技术尖刺

正式上线前必须在当前支持的 macOS CVR 版本上验证并记录（结论以测试记录或 ADR 形式入
`docs/kb/`）：

1. **(a) Merge 接受度**：CVR 的 Merge（扩展配置）能否接受本 `merge.yaml` 的
   `proxies` / `proxy-groups` / `rule-providers` / `prepend-rules` 结构，且 `prepend-rules`
   确实前插于订阅规则之前。
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
5. `shine app build clash-verge` 在控制器不可达/未配置时以清晰错误非零退出。

> CVR 侧行为（规则刷新后仍前插于订阅规则、CVR 重启后仍生效）依赖 CVR 的合成实现，`shine` 无法
> 交付该保证，归入 §8 尖刺记录，不作为验收项。

## 10. 成功指标

- 用户能在不重新导入订阅的情况下，为一个或多个 CVR 订阅添加并长期保留本地规则。
- **改规则零 CVR 交互**：日常规则变更只需 `shine task run upload_surge`（+ 可选
  `shine app build clash-verge`），无需在 CVR 内操作。
- 订阅刷新后，本地规则丢失的反馈为零。
- 不耦合 CVR 私有存储格式，也不新增后台服务、本地监听端口或自动执行的外部 hook。
