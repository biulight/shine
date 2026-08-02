# SSH 场景终端主题同步 PRD

## 1. 摘要

shine 在 Ubuntu/macOS 的 sys profile 中支持根据终端背景色选择 `bat` 主题。当前实现由远端
shell 向本地终端发送 OSC 11 查询，**在响应分片到达时会把响应的尾巴回显到用户提示符上**，
产生类似 `11;rgb:ffff/ffff/ffff` 的可见输出（根因见 §2.1）。

本 PRD 将主题同步重构为一个由 Rust 统一实现、支持多种传输路径的功能：

1. `shine ssh` 会话优先使用会话内可信的主题提示传递机制；
2. 环境变量未传入时，远端可回退到 OSC 11 查询（由 Rust 以正确的超时策略实现）；
3. 所有路径失败时静默使用远端默认主题；
4. 用户可在 `~/.shine/config.toml` 中关闭自动同步；
5. 手动同步是显式命令，不作为配置状态存在，也不要求默认暴露 shell 函数。

**收益边界（重要）**：只有 `shine ssh` 用户能吃到"不在远端发起查询"这条根除路径。普通
`ssh` 用户仍然落在 §6.2 的 OSC 回退上——对他们而言本 PRD 的收益是**同一个修复改由 Rust
实现、并被 PTY 测试锁住**，这是真收益，但不是"根除竞态"。§3 又明确不强制用户改用
`shine ssh`，因此这条边界必须写在明处，不能靠传输链的排布暗示它不存在。

## 2. 背景与问题

用户的本地终端可能根据时间或系统外观自动在浅色/深色主题之间切换。通过 SSH 登录后，
远端程序通常不知道本地当前主题，导致：

- 本地终端已经切换为浅色，但远端 `bat` 仍使用深色主题；
- 用户需要手动设置 `BAT_THEME`。

### 2.1 现有实现的缺陷（已实测，非推断）

现有读取循环（`presets/sys/{ubuntu,macos}/profile.pre.sh`）给首字节 150ms，但把**字节间**
超时降到 10ms（`read_timeout="0.01"`）。SSH 链路上响应分片是常态；一旦分片间隔超过 10ms：

1. 循环超时 `break`，此时只读到 2 字节（`\033]`）；
2. `stty` 恢复 echo；
3. 剩余字节这才到达，被原样回显到提示符。

用户看到的 `11;rgb:...` 是响应的**尾巴**——缺少开头的 `\033]` 正是因为它被循环消费掉了。

**实测数据**——用 `pty.fork()` 驱动真实循环并注入合成响应，**两个受影响平台都测了**：
Ubuntu / bash 5.3.9（`read -n` 分支）与 macOS / zsh 5.9（`read -k` 分支）。**两者行为完全一致**，
唯一差异是超时返回码（bash `142` = 128+SIGALRM，zsh `1`）：

| 响应到达方式 | 循环消费 | 耗时 | 尾巴泄漏 |
|---|---|---|---|
| 整包一次到达 | 25 B（完整） | 1ms | 否 |
| 分片，间隔 50ms | **2 B = `\033]`** | 10ms | **是** |
| 分片，间隔 5ms | 25 B | 6ms | 否 |
| 终端完全不响应 | 0 | 150ms | 否（静默跳过，正确） |

**坏的只有"分片间隔 >10ms"这一种情况**；终端不响应的路径本身是健康的，无需改动。

`read -k -t`（zsh）与 `read -n -t`（bash）在这一点上没有语义差别——**这是循环的超时策略问题，
不是 shell 差异**。因此 Ubuntu 与 macOS 两份 profile 需要同样的修复，无需分平台设计。

提交 `6f23c6b9` 曾尝试用 `stty -echo` 修复并失败：它只覆盖循环**期间**，而泄漏发生在循环
**之后**。详见 [`docs/kb/lessons.md`](kb/lessons.md) 的 2026-07-14 条目。

### 2.2 由此得出的设计取向

- 本地终端查询对应"整包到达"（实测 1ms、零泄漏），**分片根本不成立**——所以在本地查询、
  把结果传给远端是可靠的；在远端查询，永远要赌迟到字节。这是 §6.1 优先于 §6.2 的实测依据。
- Rust 化的真实收益是**能写对超时策略并且能测**（总截止时间 + 读到终止符 + PTY 集成测试
  覆盖分片），而不是"语言更安全"。同样的修复在 shell 里也做得到，只是难测、易回归。
- 残留竞态无法靠实现消除：响应晚于总预算时，仍要在某刻恢复 tty 并赌没有迟到字节。**根除只能
  靠不在远端发起查询**（§6.1）。

主题同步属于展示层能力，不应改变 SSH 认证、权限或文件传输语义，也不应要求用户必须使用
`shine ssh` 才能获得基本的 SSH 体验。

## 3. 产品目标

- 普通 `ssh <host>` 登录无需改变使用习惯。
- `shine ssh <host>` 能提供更可靠的主题提示传递路径。
- 远端 `bat` 能在登录时尽可能匹配本地终端的明暗主题。
- 不要求用户修改每台服务器的 `sshd_config` 才能使用 shine。
- 将 OSC、PTY、超时、RGB 解析和 shell 输出逻辑集中到 Rust 实现。
- profile 只负责读取配置并调用 Rust 命令，不维护复杂协议解析。
- 主题同步失败时不得阻塞登录、污染输入或使 shell 启动失败。
- 用户可以通过一个配置开关关闭自动同步。

## 4. 非目标

- 不修改本地终端窗口的真实主题；本地终端仍由 Ghostty、Terminal、iTerm2 等程序控制。
- 不保证已有 SSH 会话在本地主题变化后实时更新；同步以登录或显式命令为边界。
- 不要求所有 SSH 服务器都允许任意环境变量转发。
- 不把主题信息作为认证、授权或安全策略输入。
- 不默认安装一个新的用户可见 `shine-terminal-theme` 命令。
- 不强制用户改用 `shine ssh`。
- **不支持 tmux / screen**：OSC 查询在终端复用器下需要 DCS 透传，v1 不做。`TERM` 为
  `screen*` / `tmux*` 时直接跳过 OSC 路径，静默回退到默认主题。这类会话仍可通过
  `shine ssh` 注入（§6.1）或手动同步（§9）拿到正确主题。
- **不承诺普通 `ssh` 场景根除竞态**：普通 SSH 落在 §6.2 的 OSC 回退上，实现正确也只是把
  泄漏概率压到很低，无法归零。根除需要用户改用 `shine ssh`，而本 PRD 不强制（见上条）。
- v1 不支持 fish / PowerShell（§7）。

## 5. 用户配置

默认配置保持简单，只控制自动同步。**使用扁平键，不引入 `[sys]` 表**：

```toml
sync_terminal_theme = true    # 或 false 关闭
```

**为什么是扁平键**：`Config`（`cli/src/config/mod.rs:43-160`）是扁平 struct，唯一的嵌套表是
`[env]`。`allow_app_hooks: bool`（`:119-120`）是现成范式，落地清单已知且只需改 4 处：
字段 + `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`（`mod.rs:119-120`）、
`new_for_test` 与 `Default`（`mod.rs:220,377`）、项目层 override（`load.rs:29,132-134`）、
往返测试（`save.rs:333-342`）。新引入 `[sys]` 嵌套表是全新模式，且可能牵动 `schema_version`。

配置语义：

- `true`（缺省）：profile 登录时自动尝试主题同步；
- `false`：profile 不自动执行主题同步；
- 手动同步命令不受该开关限制，因为它是用户显式触发的操作。

**开关优先级**：环境变量 `SHINE_SYNC_TERMINAL_THEME` **覆盖**配置文件。`=0` 时无论配置为何
都不同步。这与 profile 中的既有行为一致（`profile.pre.sh:42`），也是 env-over-config 的惯例。

不引入手动同步配置状态。手动同步是用户显式执行命令的行为。

## 5.1 已发布契约（必须保持兼容）

主题同步随 **v0.38.0 已发布**，以下变量已在 `README.md:240` 与 `docs/README.zh-CN.md:240`
中作为公开契约记录，本次重构**不得丢弃**：

| 变量 | 语义 | 现状 |
|---|---|---|
| `SHINE_SYNC_TERMINAL_THEME=0` | 关闭自动同步 | 必须继续有效 |
| `SHINE_BAT_LIGHT_THEME` | 覆盖浅色主题名（缺省 `GitHub`） | 必须继续有效 |
| `SHINE_BAT_DARK_THEME` | 覆盖深色主题名（缺省 `OneHalfDark`） | 必须继续有效 |
| `SHINE_TERMINAL_THEME` | 导出的 `light`/`dark` 结果 | 语义不变 |

`shine theme sync` 的输出必须遵循同样的覆盖顺序：先解析明暗，再用
`${SHINE_BAT_LIGHT_THEME:-GitHub}` / `${SHINE_BAT_DARK_THEME:-OneHalfDark}` 的等价逻辑决定
`BAT_THEME` 的值。硬编码主题名会构成对已发布行为的回归。

## 6. 传输策略

主题同步由 Rust 统一选择传输方式，优先级如下：

```text
已有 SHINE_TERMINAL_THEME（含 shine ssh 注入）
        ↓
COLORFGBG（若终端已设置）
        ↓
远端 OSC 11 查询（兼容回退，有残留竞态）
        ↓
远端默认主题
```

`SendEnv`/`AcceptEnv` **不在传输链内**，理由见 §6.4。

### 6.1 `shine ssh` 路径（设计主线）

`shine ssh` 已经拥有本地会话上下文和远端命令包装能力。**它必须在 spawn `ssh` 之前查询本地
终端**，再把结果注入远端 shell：

```bash
SHINE_TERMINAL_THEME=light
```

**为什么本地查询是安全的**（§2.1 实测支撑）：本地查询走自己的 tty、亚毫秒 RTT，对应实测的
"整包一次到达"场景（25 字节、1ms、零泄漏），**分片根本不成立**。远端查询才要赌迟到字节。
查询失败时仅退化为不注入，不影响 `ssh` 本身。

**这条路径不能靠"本地 shell 恰好已导出该变量"**：若本地 shell 没跑过主题同步，就无从注入，
整条路径落空。因此主动查询是本路径成立的前提，不是可选优化。

该路径不依赖远端 `AcceptEnv`，也不需要远端主动向本地终端发送 OSC 查询。它是唯一能**根除**
竞态的路径，但不应成为普通 SSH 用户的强制入口（§3）。

**实现约束**：注入点是现成的——`cli/src/ssh/mod.rs:320-346` 的
`build_wrapped_remote_command` 已用扁平 `env K=V K=V ... sh -c ...` 前缀传了
`SHINE_SSH_SESSION`/`SHINE_SSH_TOKEN`/`SHINE_SSH_REMOTE_SOCK`，加第四个变量是平凡改动。
注意现有三个值是内部生成的 UUID/hex/路径，**未经引用直接插值**；主题值虽受 `light|dark`
白名单约束，仍须走 `single_quote`（同文件 `:352`），不要延续这个隐患。

### 6.2 OSC 11 回退路径

当远端没有收到 `SHINE_TERMINAL_THEME` 且自动同步开启时，Rust 命令可以尝试通过 `/dev/tty`
发送 OSC 11 查询。**普通 `ssh` 用户默认落在这条路径上**（§1 的收益边界）。

该路径需要：

- 仅在交互式 shell 和可用 PTY 中执行；
- `TERM` 为 `screen*` / `tmux*` 时直接跳过（§4 非目标）；
- 在发送查询前保存 tty 状态；
- 暂时关闭 echo；
- **使用总截止时间读到终止符为止，禁止使用字节间超时**——这是本 PRD 最关键的实现约束，
  现有实现正是违反了它才产生 §2.1 的线上缺陷。总预算见 §11；
- 恢复 tty 前先做一轮非阻塞排空，收窄在途字节窗口（**只收窄，不根除**）；
- 只接受合法的 `rgb:RRRR/GGGG/BBBB` 响应；
- 无响应、半包、非法响应或超时均静默失败；
- 无论成功、失败或异常退出都恢复原 tty 状态；
- 不把任何响应内容直接打印到用户终端。

> ⚠️ 实现者注意：不要把"关闭 echo"当成充分条件。提交 `6f23c6b9` 正是这么做的，它无效——
> 泄漏发生在读取循环**之后**。见 [`docs/kb/lessons.md`](kb/lessons.md) 2026-07-14 条目。

OSC 是兼容性回退，不是安全边界。Rust 实现应尽量降低 PTY 污染风险，但不承诺覆盖所有
终端复用器、跳板机和非标准终端实现。**即便实现完全正确，残留竞态依然存在**：响应晚于总
预算时，仍要在某刻恢复 tty 并赌没有迟到字节。根除只能靠 §6.1。

### 6.3 `COLORFGBG`

部分终端会设置 `COLORFGBG`（形如 `15;0`，分号后为背景色索引）。可用时它零成本、无 PTY
风险、无竞态，因此排在 OSC 查询之前。解析失败或变量缺失时静默跳到 OSC 路径。

### 6.4 为什么不做 `SendEnv`/`AcceptEnv`

本路径**已从传输链中移除**。原设计让本地 `ssh` 客户端用 `SendEnv` 发送主题变量、远端 sshd
用 `AcceptEnv` 接收。它同时要求三件事成立：

1. 远端 `sshd_config` 配置了 `AcceptEnv SHINE_TERMINAL_THEME` —— 与 §3"不要求用户修改每台
   服务器的 `sshd_config`"**直接矛盾**；
2. 用户手动改了本地 `~/.ssh/config` 加上 `SendEnv`；
3. 本地 shell 里恰好已经有 `SHINE_TERMINAL_THEME`（否则发的是空值）。

三个前提都满足的用户，直接用 `shine ssh` 即可获得更好的结果且零配置。为一条"只有配置到位
才生效、而配置到位就没必要"的路径增加代码与测试面，不划算。

用户若坚持要用，这仍是标准 SSH 能力，自行配置即可生效——因为 §6 的第一优先级就是"已有
`SHINE_TERMINAL_THEME`"。**shine 不为此提供任何自动化，也不做可用性探测。**

### 6.5 默认回退

所有传输路径都不可用时：

- 不打印错误；
- 不修改用户输入；
- **保留用户已经设置的 `BAT_THEME`**；
- 否则使用 shine 的远端默认主题（§5.1 的覆盖顺序：`SHINE_BAT_DARK_THEME` 缺省
  `OneHalfDark`，`SHINE_BAT_LIGHT_THEME` 缺省 `GitHub`）。

> **这是行为变更**：现有实现**无条件覆盖** `BAT_THEME`（`profile.pre.sh:28,31`）。改为保留
> 用户已设置的值是有意为之的改进，须在 §12 与 README 中标注。用 `SHINE_TERMINAL_THEME` 是否
> 已存在来区分"用户设的"与"shine 在父 shell 设的"，嵌套 shell 因此天然跳过。

## 7. Rust 命令接口

新增统一命令：

```text
shine theme sync [--quiet]
```

**没有 `--shell` 标志**：shell 类型从 `config.shell_type` 自动派发，与既有的
`shine env secret export` 先例一致（`cli/src/env/commands.rs:173-194` 的 `handle_export` 正是这么做的，
它也没有 `--shell`）。原设计的 `--shell bash|zsh|fish|powershell` 与该先例相反，且其中 `fish`
对 profile 路径是死路——sys profile 里 fish 直接落进 `unsupported_shell: true`
（`cli/src/sys/profile_blocks.rs:38-78`），macOS 更是 zsh 硬编码（`:28-36`）。

**v1 范围：仅 bash / zsh**（即 sys profile 实际支持的集合）。Windows/PowerShell 与 fish 单列
后续，不在本 PRD 内。§6.2 的 OSC 路径本身也是 unix-only。

命令职责：

- **只读**加载 shine 配置——不得创建配置状态。`Config::load_or_init()` 会写盘（见 AGENTS.md），
  而本命令在**每次交互式 shell 启动时**都会跑，必须走只读加载路径；
- 按 §6 的优先级解析主题：已有 `SHINE_TERMINAL_THEME` → `COLORFGBG` → OSC 11 查询；
- 判断 `light` / `dark`；
- 按 §5.1 的覆盖顺序决定 `BAT_THEME`；
- 输出 shell-safe 的环境变量赋值代码；
- 将诊断信息输出到 stderr，`--quiet` 时隐藏非必要诊断；
- 无法确定主题时返回成功但输出为空，避免破坏 profile 启动。

**不得触发网络更新检查**（`update_check::maybe_notify` 必须跳过）。

示例输出：

```bash
export SHINE_TERMINAL_THEME='dark'
export BAT_THEME='OneHalfDark'
```

由于子进程不能直接修改父 shell 环境，profile 或手动 wrapper 需要使用 `eval` 应用输出：

```bash
eval "$(shine theme sync --quiet 2>/dev/null)"
```

**Quoting 必须复用现有实现**，不得新写第 4 份：`cli/src/env/commands.rs:220-240` 已有
`format_env_export` + `posix_shell_quote` / `fish_quote` / `powershell_string_quote`。
（仓库目前已有 3 份独立的 `single_quote`：`shell_quote.rs:9`、`ssh/mod.rs:352`、
`env/commands.rs:230`——不要再添一份。）

## 8. Profile 集成

Ubuntu/macOS 的 managed `pre` profile 只保留薄调用逻辑：

```bash
if [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" != "0" ]] &&
   command -v shine >/dev/null 2>&1; then
  eval "$(shine theme sync --quiet 2>/dev/null)"
fi
```

shell 类型由 Rust 侧从 `config.shell_type` 自动派发（§7），profile 不传。profile 不直接包含：

- OSC 响应状态机；
- RGB 解析；
- tty 状态恢复逻辑；
- 多种传输方式的优先级判断。

如果 `shine` 二进制不存在、版本过旧或命令失败，profile 必须静默跳过，不能阻止用户打开
shell。**版本过旧是安全的**：旧二进制遇到未知子命令时 clap 会把错误写到 stderr（被
`2>/dev/null` 吞掉）并让 stdout 为空，`eval ""` 是空操作。

## 9. 手动同步

手动同步是显式行为，不作为配置状态存在。

可以提供可选的 `shell/utils` wrapper，复用 `shine-env-export` 的设计：

```bash
#!/bin/bash
eval "$(shine theme sync "$@")"
```

该 wrapper：

- 使用 `needs_source = true`；
- 只有用户主动安装 `shell/utils` 后才会暴露；
- 不会默认增加 shell 函数或命令；
- 与自动 profile 共用 Rust 实现；
- 不重复实现 OSC 协议。

示例：

```bash
shine shell install utils
shine-theme-sync
```

如果用户没有安装该可选 wrapper，仍可直接使用：

```bash
eval "$(shine theme sync)"
```

## 10. 安全与失败策略

- `SHINE_TERMINAL_THEME` 只用于展示层配置，不得用于权限判断。
- 环境变量内容必须限制为 `light` 或 `dark`，其他值视为无效。
- shell 输出必须复用 `cli/src/env/commands.rs:220-240` 的 quoting 实现（§7），不能直接拼接
  未经转义的值，也不要新增第 4 份 `single_quote`。
- `shine ssh` 注入的主题值须经 `single_quote`（`ssh/mod.rs:352`）后再插入 `env K=V` 前缀，
  不要延续现有三个变量未经引用直接插值的做法（§6.1）。
- OSC 响应必须经过格式和长度验证，禁止把原始响应回显给用户。
- tty 状态必须使用 guard/清理路径恢复，避免 shell 登录后永久关闭 echo。**但要清楚：恢复 tty
  本身就是泄漏窗口的开端**（§2.1），guard 保证的是不把终端弄坏，不是不泄漏。
- 查询失败、终端不支持 OSC 时均静默回退。
- 不接受来自远端环境变量的命令、路径或 SSH 参数。
- `shine ssh` 注入的主题值仍然只是展示提示，不改变远端命令权限。

## 11. 验收标准

### 时间预算（必须可判定）

| 场景 | 预算 |
|---|---|
| `SHINE_TERMINAL_THEME` 已传入（`shine ssh` 路径） | ≤ 30ms（不得有任何 tty 交互） |
| `COLORFGBG` 命中 | ≤ 30ms |
| OSC 查询，终端正常响应 | ≤ 50ms |
| OSC 查询，终端不响应 | **总预算 200ms 上限**，之后静默跳过 |

"不会明显阻塞"不是验收标准。总预算是**总截止时间**，不是字节间超时（§6.2）。

### 功能

- 本地深色终端登录远端后，远端 `bat` 使用深色主题；
- 本地浅色终端登录远端后，远端 `bat` 使用浅色主题；
- `shine ssh` 不依赖 `AcceptEnv` 即可传递主题提示；
- `shine ssh` 在本地 shell 未预设 `SHINE_TERMINAL_THEME` 时，仍能主动查询本地终端并注入；
- 未走 `shine ssh` 时，OSC 回退可以工作或安全失败；
- 用户已设置的 `BAT_THEME` 被保留（§6.5，行为变更）；
- `SHINE_BAT_LIGHT_THEME` / `SHINE_BAT_DARK_THEME` 覆盖仍然生效（§5.1）；
- 自动同步关闭后，profile 不发送 OSC 查询；
- 手动同步可以在自动同步关闭后显式执行；
- 未安装可选 wrapper 时，默认 profile 不暴露新的 shell 函数。

### 稳定性

- **响应分片到达时不会出现 `11;rgb:...` 等残留输出**（见下方测试矩阵）；
- 查询后 tty echo、canonical mode 等原始状态保持不变；
- bash、zsh 输出均可正确应用；
- 无 `shine` 二进制或旧版本二进制时 profile 能静默启动。

### 测试

**PTY 集成测试矩阵（必须项）**——直接复刻 §2.1 的实测场景，逐条比对：

| 响应到达方式 | 期望 | 现有实现 |
|---|---|---|
| 整包一次到达 | 完整解析，无泄漏 | ✅ 通过 |
| **分片，间隔 50ms** | **完整解析，无泄漏** | ❌ **失败**（消费 2B，尾巴泄漏） |
| 分片，间隔 5ms | 完整解析，无泄漏 | ✅ 通过 |
| 终端完全不响应 | 静默跳过，≤ 200ms | ✅ 通过 |

> ⚠️ "分片间隔 50ms" 这一行是本 PRD 存在的原因，也是**唯一挡住该缺陷回归的东西**（本次
> 未做 shell 侧热修，全部押在 Rust 实现上）。它必须是硬性 CI 用例，不得降级为手工验证。
> 分片间隔应参数化，至少覆盖一个远大于任何字节间超时的值。

其余测试：

- Rust 单元测试覆盖 RGB 解析、亮度判断、非法响应和 shell quoting；
- Rust 测试覆盖传输优先级（已有变量 → `COLORFGBG` → OSC）和默认回退；
- Rust 测试覆盖 `BAT_THEME` 保留与 `SHINE_BAT_*_THEME` 覆盖（§5.1 的已发布契约）；
- PTY 测试覆盖半包响应、非法响应、异常退出后的 tty 恢复；
- `shine ssh` 集成测试覆盖主题变量注入及其 `single_quote`；
- 手动 wrapper 测试确认 `needs_source = true` 且未安装时不默认暴露。

**删除的测试项**：原"profile 嵌入资源测试确认只包含薄调用，不包含 OSC 解析实现"是对实现
细节做字符串断言，改一次 profile 就要改一次测试。`cli/src/sys/commands.rs:1552-1575` 现有的
同类断言更糟——它正在断言 `stty -echo` 这个**已证无效的修复**，把错误固化成了测试，应随本次
重构一并清理或改造（见 §12）。

## 12. 发布与兼容性

### 12.1 已发布契约

主题同步随 **v0.38.0 已发布**。§5.1 列出的四个变量都是 `README.md:240` 记录的公开行为，
必须继续有效。丢弃任何一个都是回归。

`BAT_THEME` 的保留语义（§6.5）是**有意的行为变更**：现有实现无条件覆盖。README 与
`docs/README.zh-CN.md` 的对应段落须同步更新。

### 12.2 迁移（取消热修后，这是让线上停止复现的唯一动作）

由于本次不做 shell 侧热修，**迁移本身承担了修复职责**：旧 OSC 块只要没被真正移除，
§2.1 的缺陷就原样存活。这不是整洁性问题，是正确性问题。

迁移面对两个叠加风险：

1. **合并冲突会弄坏登录 shell**。`~/.zshrc` / `~/.bashrc` 里的 sentinel 块只是 3 行 loader
   （`cli/src/sys/profile_blocks.rs:211-253`），真正被三方合并的是
   `~/.shine/profile/<os>-sys.<phase>.sh`（`cli/src/sys/profile.rs`）。把 40 行 OSC 块换成
   4 行调用，对改过该文件的用户会留下冲突标记——而它是登录时 `source` 的，**shell 直接起不来**。
2. **替换不彻底 = 缺陷存活**。

因此迁移不能停留在"应替换或合并旧的 OSC 实现"，需要写死机制：按 sentinel 精确定位旧函数块
并**整体替换**，而不是逐行三方合并。实施前须先读 `sys/profile.rs` 的合并实现确认可行性。

> 抽样核查（2026-07-14，一台线上 Ubuntu）：该机 `.pre.sh` 与 `.pre.base.sh` 逐字节相同、
> 0 个冲突标记，重装路径正常。但那是**未改过 profile 的机器**，不能据此推广。

`cli/src/sys/commands.rs:1552-1575` 的嵌入资源断言目前锁定的是 `stty -echo` 这个无效修复。
它可以改造成迁移的闸门：断言 profile **不含**旧 OSC 实现（尤其不含字节间超时）。

### 12.3 其他

- 新版本 profile 必须兼容没有 `sync_terminal_theme` 的旧 `config.toml`，缺省按自动同步处理。
- 旧 profile 中的 `SHINE_SYNC_TERMINAL_THEME=0` 继续有效。
- 新 Rust 命令不可用时，旧 profile 不得因找不到命令而失败（§8 已说明为何安全）。
- 文档需要明确说明：终端窗口主题始终由本地终端控制，shine 只同步远端工具的主题提示。

## 13. 未决问题

- 是否需要 `unknown` 等内部检测结果状态。

### 已定案（原未决问题）

| 原问题 | 结论 | 依据 |
|---|---|---|
| 配置放 `[sys]` 还是 `[terminal]` | **都不放，用扁平键** | §5：`Config` 是扁平 struct，`allow_app_hooks` 是现成范式 |
| OSC 回退是否默认启用 | **默认启用**，但 `TERM=screen*\|tmux*` 跳过 | §6.2；§2.1 实测表明"不响应"路径是健康的 |
| `shine ssh` 是否应主动查询本地终端 | **必须**，这是 §6.1 成立的前提 | §6.1：否则本地无变量时整条路径落空 |
| tmux/screen 是否需要提示 | **统一静默跳过**，列为非目标 | §4 |
| macOS 的 zsh `read -k -t` 是否与 bash 分支同缺陷 | **是，行为完全一致** | §2.1：macOS/zsh 5.9 实测复现，仅返回码不同 |
