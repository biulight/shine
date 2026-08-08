# SSH 会话按需密钥代理 PRD

## 1. 背景

`shine ssh --with-secret KEY` 在本机解密 `KEY_SECRET` 后，将明文作为远端 SSH
命令的环境变量注入。传输受 SSH 保护，但明文会在整个远端 shell 会话及其子进程树中
可用，并可能出现在本机 SSH 子进程参数、远端进程环境或误日志中。

另一方面，`shine env secret seal` 产生的 GPG/age 密文可以安全保存在远端工作区；其
recipient 可对应仅由本机 YubiKey 持有的私钥。本功能让远端保留密文、让本机保留私钥，
仅在远端受管命令真正启动时通过当前 `shine ssh` 会话请求本机解密并短时注入。

本文将此能力称为 **SSH Secret Broker**。它是 `shine ssh` 的会话能力，而不是现有
`shine env proxy` 的扩展：后者是本地 PATH shim，负责为本机子进程注入允许的环境变量，
没有网络协议或远端授权含义。

## 2. 产品目标

- 远端可保存、提交和同步由 `shine env secret seal` 生成的密文，永不保存解密私钥或
  YubiKey 凭据。
- 用户以 `shine ssh` 建立会话后，远端 `shine env run` 可为**一次**目标命令申请已授权的 secret。
- 本机使用现有 tag-routed GPG/age 后端解密；GPG/YubiKey 的 PIN、touch 与 agent 交互
  始终在本机发生。
- secret 不进入 SSH 登录 shell、远端命令参数、远端持久化配置或 Shine 日志；目标程序
  退出后不保留 Shine 侧的明文或授权状态。
- 请求仅可使用本次 `--allow-secret` 授权，或已批准的“SSH target + workspace/source 摘要 + mode + argv”项目策略。
- SSH 本身、普通远端命令、现有 `--with` 与 `--with-secret` 行为保持兼容。

## 3. 非目标与安全边界

- 本功能不使已被 root/管理员或等效权限攻陷的远端主机变得可信。目标程序需要 secret
  时仍会在远端短暂拥有明文；有权限读取该进程内存、环境或 I/O 的攻击者可能窃取它。
- 不做远端私钥、YubiKey、gpg-agent 或 SSH agent 转发，也不在远端建立常驻守护进程。
- 不向任意远端命令、任意 secret 名或任意用户提交的密文提供“解密 API”。
- MVP 不支持 Windows remote shell：当前 `shine ssh --remote-shell windows` 不创建反向
  转发或 `shine local` 控制通道。Windows 远端支持留作后续独立设计。
- 不替代 OIDC、工作负载身份、云 IAM 或 Secret Manager。对于可改造的服务，短期凭证
  仍是优先方案。

## 4. 用户体验与配置

### 4.1 三种 secret 使用模式

| 模式 | 本机命令 | 远端使用 | 适用场景与边界 |
| --- | --- | --- | --- |
| 即时注入（既有能力） | `shine ssh --with-secret API_TOKEN --with-secret NPM_TOKEN dev` | 登录 shell/命令直接继承变量 | 最快，支持单个或多个变量；连接建立时即解密，明文会出现在 SSH 命令和远端会话环境。保留原样，不走 broker。 |
| 按命令 broker | `shine ssh --secret-broker --allow-secret API_TOKEN --allow-secret NPM_TOKEN dev` | `shine env run --no-workspace --secret-broker --secret API_TOKEN --secret NPM_TOKEN -- bun run build` | 一次性、非项目级命令；变量只注入该子进程。默认每次请求在本机确认/YubiKey 解密。 |
| workspace broker | `shine ssh --secret-broker dev` | `shine env run --mode development --secret-broker -- bun run build` | 项目级环境；使用本机策略库精确匹配 workspace/source 摘要、mode 和 argv。默认逐次确认；只有显式信任整个远端会话时才自动批准。 |

`--with-secret` 继续支持重复使用和 `KEY=ALIAS` 别名，它是用户明确选择的便利模式；broker
不会改变其 argv/environment 暴露边界。`--allow-secret KEY[=ALIAS]` 仅创建当前 SSH 会话的
单项请求能力，不读取或写入策略库，且不能以 `*_SECRET` 后缀作为输入。远端必须以重复的
`--secret KEY[=ALIAS]` 显式请求每个变量；它不是授权本身，本机仍要求与 `--allow-secret`
一一匹配。显式请求避免“把当前会话所有允许 secret 全部注入”的宽松默认，并让确认提示、
审计与目标子进程的最小变量集保持可见。MVP 中 `KEY` 是公开的本机 key 名；后续可将其替换
为本机策略映射的稳定 secret reference，从而不暴露存储键名。按命令 broker **只**解密
`KEY_SECRET`；若密文不存在必须拒绝并提示改用既有、显式的明文注入路径，绝不回退发送 `KEY`。

对按命令 broker，本机确认提示必须显示 SSH host、目标 argv、每个 source key/alias；取消、
重复 alias、未知/未允许 key 或会话结束均拒绝。按命令 broker **始终**逐次本机确认，禁止
自动批准：远端会话能力可能被同账号进程复用。项目级请求默认也逐次确认；只有本机显式传入
`--trust-remote-session` 才可对命中内容策略的请求自动批准。该参数表示信任**整个远端会话及
同账号进程**，而不是证明请求来自目标项目；帮助、启动输出和首次释放提示必须明确这一点。

`--allow-secret` 在会话建立时冻结每个允许项的 source key、目标 alias、密文字节及 SHA-256；
后续请求只解密该快照。会话中途修改本机配置不会悄然改变本次会话实际释放的值。

### 4.2 远端密文

用户在本机或 CI 中，用本机持有私钥对应的公开 recipient seal 工作区环境文件：

```bash
shine env secret seal --workspace shine.workspace.toml
```

生成的密文随项目位于远端。密文需使用现有 `age:<base64>` 或未加标签的 GPG 格式；解密
始终由密文标签决定，而不依赖远端配置的默认 backend。

### 4.3 建立允许的项目级会话

默认策略库位于本机 `<shine_dir>/ssh-secret-broker.toml`。它是可触发 secret 释放的授权数据库，
只能从本机全局层加载：不得受项目配置、preset overlay、环境变量中的外部策略路径或远端请求
覆盖。Unix 上文件必须由当前用户拥有、权限不宽于 `0600`；所有平台均拒绝 symlink，使用
同目录临时文件 + 原子替换更新，并在替换前后复核 owner/类型。用户通过不带路径的开关显式为
当前 SSH 会话启用 broker：

```bash
shine ssh --secret-broker dev
```

默认策略文件可包含多个命名条目；它只存在于本机，且不含明文 secret。每条将 SSH target、
项目的**环境定义版本**、允许的 mode/命令和密文版本绑定在一起，例如；workspace/source
摘要不覆盖项目代码、依赖或目标二进制，不能称为整个项目的内容身份：

```toml
version = 1

[[policy]]
name = "dev-api-build"
ssh_target = "dev"
project = "acme-api" # 仅用于本机输出与审计
workspace_sha256 = "<sha256>"

[[policy.allow]]
mode = "development"
argv = ["bun", "run", "build"]
release = ["DEPLOY_TOKEN", "REGISTRY_TOKEN"]
sources = [
  { path = "env/development.toml", sha256 = "<sha256>", declared_secrets = ["DEPLOY_TOKEN", "REGISTRY_TOKEN"] },
]
```

`workspace_sha256` 和每个 source 的摘要由用户在本机审阅、登记；实际密文 payload 的摘要
也在其中记录或由 source 摘要覆盖。它们共同标识一个项目版本，不依赖远端部署路径，因而
同一策略可用于容器、蓝绿发布或不同目录。`remote_workspace` 可作为可选的精确路径约束，
用于防止运维误连到同内容的错误部署，但不作为核心授权条件。

建立 SSH 时本机加载整个策略库；每次远端发出 `env run --secret-broker` 请求后，以 SSH
target、workspace/source 摘要、mode、完整 argv 和可选路径约束进行精确匹配。**恰好一条**
策略匹配才为该请求创建一次性、不可复用的授权 lease；零条或多条均拒绝并展示可操作的本机
诊断，不解密任何内容。这样同一 SSH 会话可在不同项目目录中执行各自已批准的命令，但每个
请求仍独立匹配和计数。MVP 不支持 host、路径、argv 或 digest 通配符，避免“最宽松规则意外
获选”。`ssh_target` 是用户传给 OpenSSH 的 target/alias，仅用于策略选择；主机真实性仍完全由
OpenSSH known_hosts、HostKeyAlias 和既有严格主机密钥检查保证。实现可记录 `ssh -G` 得到的
effective user/hostname/port 供确认和审计显示，但不得把 alias 本身描述为主机认证。未传
`--secret-broker` 时不创建 broker，既有 SSH 行为不变。

`--secret-broker-policy <FILE>` 可作为可重复的高级覆盖参数，用于临时策略或测试；它与默认
策略库合并后仍使用同样的 owner/权限/symlink 检查和唯一精确匹配规则。只有本机命令行可
指定该路径；环境变量、项目配置和远端请求均不能指定。

#### 多策略完整示例

下面是一份本机 `~/.shine/ssh-secret-broker.toml` 的示例。每个 `[[policy]]` 是一个独立
项目版本；同一 policy 下可有多个 `[[policy.allow]]`，分别放行不同的 mode/命令组合。
示例中的 64 位十六进制字符串是 SHA-256 占位符，实际值由登记/审阅流程生成，不能照抄。

```toml
version = 1

# SSH target/alias 为 dev 的 API 项目：允许 development build 和 test。
[[policy]]
name = "dev-api"
ssh_target = "dev"
project = "acme-api"
workspace_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[policy.allow]]
mode = "development"
argv = ["bun", "run", "build"]
release = ["API_TOKEN", "NPM_TOKEN"]
sources = [
  { path = "env/base.toml", sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", declared_secrets = [] },
  { path = "env/development.toml", sha256 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc", declared_secrets = ["API_TOKEN", "NPM_TOKEN"] },
]

[[policy.allow]]
mode = "development"
argv = ["bun", "test"]
release = ["API_TOKEN"]
sources = [
  { path = "env/base.toml", sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", declared_secrets = [] },
  { path = "env/development.toml", sha256 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc", declared_secrets = ["API_TOKEN", "NPM_TOKEN"] },
]

# 同一 SSH 主机上的另一个项目：摘要、source 与允许的命令均独立。
[[policy]]
name = "dev-web-release"
ssh_target = "dev"
project = "acme-web"
workspace_sha256 = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
remote_workspace = "/srv/releases/acme-web/shine.workspace.toml" # 可选的运维约束

[[policy.allow]]
mode = "production"
argv = ["bun", "run", "build"]
release = ["SENTRY_AUTH_TOKEN"]
sources = [
  { path = "env/production.toml", sha256 = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", declared_secrets = ["SENTRY_AUTH_TOKEN"] },
]

# 不同 SSH host 的部署项目。
[[policy]]
name = "staging-worker-migrate"
ssh_target = "staging"
project = "acme-worker"
workspace_sha256 = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"

[[policy.allow]]
mode = "staging"
argv = ["bun", "run", "migrate"]
release = ["DATABASE_URL"]
sources = [
  { path = "env/staging.toml", sha256 = "1111111111111111111111111111111111111111111111111111111111111111", declared_secrets = ["DATABASE_URL"] },
]
```

例如，远端 `dev` 上的 `acme-api` 执行 `bun run build` 时，只会匹配第一个
`[[policy.allow]]` 并返回两个 secret；执行 `bun test` 匹配第二个，验证相同 source 的完整
`declared_secrets` 后仅返回 `release = ["API_TOKEN"]`。若把两条 allow 写成完全相同的匹配
条件，本机必须拒绝，而不是任选其一。

#### 策略生成、远端比对与摘要

默认且推荐的登记流程从本机可信副本直接生成并写入策略：

```bash
# 本机：从本地、可信的项目副本读取 workspace/source 原始 bytes 并写入策略。
shine env broker policy add \
  --name dev-api-build \
  --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development \
  --release API_TOKEN \
  --release NPM_TOKEN \
  -- bun run build
```

该命令自行解析 workspace 和 mode 的有效 source，计算摘要并写入默认策略库；用户不需要
手工复制 hash。若本机没有 checkout，等价的可信输入是由已钉住的发布密钥验证通过的 manifest
（包含 workspace/source 的内容摘要）。这是最简单的日常流程。

策略管理同时提供 `policy list`、`policy info <NAME>`、`policy diff <NAME> --workspace ...`、
`policy update <NAME> ...` 和 `policy remove <NAME>`。`add` 遇到重名或相同匹配条件时拒绝并
提示使用 `diff/update`；`update` 先显示旧/新摘要、argv 与 release 差异，要求本机确认后原子
替换。argv 始终精确匹配，不引入通配符；命令变化通过 `diff/update` 降低维护成本。

对于用户明确确认可信的远端（例如个人受管服务器或已验证的短期部署环境），也支持从远端
元数据登记并写入：

```bash
# 本机：明确声明信任远端本次报告的 workspace/source 内容。
shine ssh --secret-broker-enroll --trust-remote-metadata dev

# 远端项目目录：只描述有效 workspace/source，不执行 build，也不解密。
cd /srv/acme-api
shine env broker describe --mode development \
  --release API_TOKEN --release NPM_TOKEN \
  -- bun run build
```

本机显示即将写入的 SSH target/effective endpoint、项目路径、mode、argv、source 路径、
declared/release secret 名和摘要，要求一次
本地交互确认后才写入新策略；同名或重叠条目必须显式选择更新，绝不静默覆盖。该模式不调用
GPG、age 或 YubiKey，也不会释放 secret。`--trust-remote-metadata` 的名称必须保留，且帮助与
确认提示必须说明：它把远端报告当作策略真源，只适用于用户已在带外确认完整性的主机。

对于未确认可信的远端，可用 inspect 模式提供**未可信候选**，仅用于诊断或与本机策略比较：

```bash
shine ssh --secret-broker-inspect dev
# 远端：shine env broker describe --mode development --release API_TOKEN -- bun run build
```

inspect 永远不释放 secret、不会创建或修改策略。它显示远端实际提供的 bytes 摘要与本机策略
的差异，以帮助用户决定是从本地可信 checkout 重新生成策略、使用明确的 trusted enrollment，
还是停止该远端会话。

运行时远端必须传送其读取的 sealed source 原始 bytes（受大小上限保护），由**本机**重新
计算摘要后与策略比对，而不是信任远端报出的摘要字段。本机验证后返回与这些 bytes 对应的
变量映射；远端 `env run` 必须只使用这份内存映射启动目标子进程，禁止在获批后重新读取 source
或 cache。此比对能防止意外配置漂移或普通篡改时释放 secret；它不能证明已被 root 控制的远端
会把 secret 只交给声称的项目。即使策略来自本地可信副本，也不能抵抗已被 root 控制的远端在
获得明文后外泄；这类场景需要远程度量/硬件证明、机密计算，或完全避免向该主机释放长期 secret。

手工排错时可使用系统命令生成单文件摘要：macOS 用 `shasum -a 256 FILE`，Linux 用
`sha256sum FILE`；正式策略始终以本机可信真源计算的原始 bytes 为准。

### 4.4 远端项目级 `env run` 流程

对需要整个 workspace 环境的命令，不应用单一命令 shim；远端 `env run` 应显式请求当前
SSH 会话的 broker：

```bash
# 本机：启用 broker；从本机默认策略库自动匹配 dev-api-build。
shine ssh --secret-broker dev

# 远端项目目录：自动发现最近的 shine.workspace.toml。
cd /srv/acme-api
shine env run --mode development --secret-broker -- bun run build
```

远端 `env run --secret-broker` 按现有规则从当前目录向上发现 `shine.workspace.toml`；若
不在项目目录，也可显式指定：

```bash
shine env run --workspace /srv/acme-api/shine.workspace.toml \
  --mode development --secret-broker -- bun run build
```

它在一次读取中保留 workspace/source 原始 bytes，向本机发送这些 bytes、source 相对
workspace 的路径、mode、完整 argv 及其中声明的 secret 名，但不在远端调用
`decrypt_secret`。若策略设有可选 `remote_workspace`，请求还须报告的绝对路径与其相符；
本机自行计算摘要，且只有在所有启用条件逐项匹配策略文件时才继续。

本机只接受与策略中的 workspace/source 摘要、mode、完整 argv、source 相对路径和完整
`declared_secrets` 一致的请求；若声明了路径约束还必须匹配该路径。解密后先验证 payload
完整 key 集，再仅返回该 allow 的 `release` 子集。远端将返回的 secret 与**同一次读取并仍
保留在内存中的** plain 值合并，只注入 `bun run build` 的直接子进程；获批后不得重新读取
workspace、source 或 cache。broker 模式**不读取或写入**现有 workspace env cache，以避免
TOCTOU、明文缓存或需要远端私钥的隐式解密路径。

### 4.5 交互

- 默认模式为每次 secret 释放均在本机显示 SSH target/effective endpoint、策略名、经过安全
  转义的 argv 与 secret reference，并等待用户确认；GPG/YubiKey 随后按自身策略要求 PIN/touch。
- `--trust-remote-session` 仅可显式允许命中内容策略的项目请求自动释放；按命令 broker 始终
  确认。输出必须提示：内容摘要不认证调用者，同账号进程可能复用会话能力。
- 请求拒绝、摘要不匹配、会话过期、用户取消或本机解密失败时，远端 `env run` 以非零状态退出，
  不启动目标程序。

#### 本机 TTY 确认

交互式 SSH 中 OpenSSH 持有并通常将本机 TTY 切到 raw mode。Unix MVP 必须由父级 `shine ssh`
协调确认：收到完整请求后暂停 SSH 子进程，保存当前 termios，恢复启动 SSH 前的 canonical/echo
状态，通过 `/dev/tty` 完成本机确认和 GPG/pinentry，再恢复 raw termios 并继续 SSH 子进程，
最后发送响应。确认期间远端会暂停，这是预期行为；任一步失败都拒绝请求并恢复终端/SSH。

实现前必须用真实交互式 SSH 尖刺验证 macOS/Linux 上的 Ctrl-C、窗口 resize、TTY 与 GUI/TTY
pinentry、请求超时及异常恢复。无本机 TTY 时，所有需要确认的请求直接失败并给出操作建议；
只有已显式传入 `--trust-remote-session` 且命中项目策略的非交互式请求可以继续。

所有显示字段均视为远端不可信输入：协议限制字符串/argv/source 数量及总字节数，拒绝 NUL
和非预期控制字符；本机 UI 以带界限、不可解释 ANSI 的转义形式显示，绝不把远端字符串直接
写入 TTY。超限或非法输入在策略匹配、显示和解密前拒绝。

## 5. 技术方案

```text
远端：shine env run --secret-broker
          │  DirectRequest { session, argv, secret refs, nonce }
          │  或 WorkspaceRequest { session, workspace/source bytes, mode, argv, nonce }
          ▼
SSH 反向 Unix socket / Windows 后续 loopback 通道
          ▼
本机：shine ssh secret-broker agent
      会话策略校验 → 本机确认 → YubiKey/GPG 或 age 解密
          │  一次性 SecretResponse
          ▼
远端 env run：仅用本机验证后的内存映射注入 → exec 目标命令
```

- 基于现有 `shine ssh` 会话目录、反向转发和协议帧扩展一个 `SecretRequest` / `SecretResponse`
  族；它与文件传输使用同一会话生命周期，但使用独立请求类型、版本协商和明确的最大消息
  长度。协议在分配大块内存、显示或解析前校验字段长度、argv/source 数量、总 source bytes、
  UTF-8/NUL/控制字符规则；无效请求不得进入确认或解密路径。
- 会话令牌只能识别当前 SSH 会话，**不构成释放 secret 的授权**。每次请求还必须匹配本机
  策略或本次 `--allow-secret` 授权、一次性 nonce 和请求次数限制；direct request 永远要求
  本机交互确认，workspace request 才可能获得自动批准。
- 本机仅对批准条目中由本机重新计算并匹配的 source/payload 摘要解密。不匹配立即拒绝且
  不调用 GPG/age，避免成为任意密文解密服务。
- `env run --secret-broker` 在远端只负责解析 workspace、mode 与非敏感 metadata；sealed
  payload 的解密与任何 cache 读写均由 broker 路径替代。它必须在启动目标子进程前完成
  source `declared_secrets` 与解密 payload 完整 key-list 的一致性校验，再按 `release` 过滤，
  沿用本地 `env run` 的完整性规则。
- 采用现有 `secret::decrypt_secret(ciphertext, config.age_identities())`，保持 GPG/YubiKey、
  age、Touch ID 和 tagged-ciphertext 路由的一致性。密文、明文、PIN、recipient、token 及
  未净化远端字符串不得写入日志或错误上下文。审计仅记录时间、SSH target/effective endpoint、
  策略名、argv 的稳定摘要、secret reference、交互/自动批准方式和结果；不记录值或原始 argv。
- SSH 已提供客户端-服务器传输加密和主机认证；在该 socket 上再次封装“二次加密”不提升
  远端端点可信度。实现重点是避免 argv、登录环境和磁盘暴露，而非自创传输加密。
- 回包通过已认证的会话通道发送。MVP 的 `env run` 仅以精确 allow-list 的环境变量注入其
  直接子进程，绝不导出到父 shell，也不占用或改写目标命令 stdin。stdin/匿名 FD 传递需要
  独立的显式接口和应用消费契约，留作后续能力。
- SSH 退出、Ctrl-C、代理连接断开、请求超时或目标进程退出时，清除本机会话策略、nonce、
  明文缓冲与远端临时状态。实现使用可清零的 secret buffer 并 best-effort zeroize，但不宣称
  能清除已复制到子进程环境、操作系统或第三方 GPG/pinentry 的所有内存。任何失败不得触发
  重试式重复解密。

## 6. 授权与威胁模型

| 威胁/情形 | 设计响应 | 剩余风险 |
| --- | --- | --- |
| 远端磁盘或 Git 历史泄露 | 仅存 seal 密文，本机保留私钥 | recipient 私钥泄露仍可解密历史密文 |
| shell 历史、argv、全会话环境泄露 | 不用 `--with-secret`；按命令短时注入 | 目标程序的环境/I/O 仍可能暴露 |
| 远端任意程序索取 secret | direct request 逐次确认；workspace request 匹配 target/workspace/source/argv 策略 | 摘要不认证调用者；`--trust-remote-session` 等价于信任同账号远端进程 |
| 替换远端密文为攻击者输入 | 本机重算 source/payload 摘要，拒绝任意解密请求 | 已批准内容可被重放，受一次性 nonce/次数限制 |
| 远端终端提示注入 | 限长、拒绝控制字符、本机转义显示 | 用户仍需核对 target、策略和 secret reference |
| 本机策略库被替换 | 全局固定路径、owner/权限检查、拒绝 symlink、原子更新 | 本机同账号/特权进程仍在信任边界内 |
| 远端 root 或目标程序被攻陷 | 不宣称防护；建议短时凭证/OIDC | 远端使用明文时可被窃取 |
| 本机同用户/特权监控 | 不经 ssh argv 注入；最小化内存生命周期 | 能监控本机解密进程者仍是高权限风险 |

远端“命令限制”与 workspace/source 摘要主要防止误用、配置漂移和低权限误调用，不能证明
请求来自该目标程序，也不能作为抵御同账号恶意进程或远端 root 替换 `env run`、项目代码或
目标二进制的可信执行证明。需要防远端主机失陷时，应采用服务侧短期凭证或可验证的远程度量/
证明机制；这不属于 MVP。

## 7. 验收标准

1. 在远端仅保存 sealed 环境文件且无 GPG/age 私钥时，`shine env run --secret-broker` 能在
   本机确认和 YubiKey touch 后成功启动允许的目标命令。
2. 远端登录 shell、`ps` 可见的命令参数、Shine 管理的文件和 Shine 日志均不包含 secret
   明文；目标程序仅能从其精确 allow-list 的环境变量读取获准值，stdin 保持原样。
3. 未传 `--secret-broker`、请求未知 secret、请求 SSH target 不匹配、密文摘要不匹配、重用
   nonce、超过允许次数和会话结束后的请求均被拒绝，且本机不调用解密后端。
4. GPG/YubiKey、age 普通 identity 和 age Secure Enclave identity 均沿用现有密文标签路由；
   backend 默认值改变不会影响旧密文。
5. 远端执行 `shine env run --mode development --secret-broker -- bun run build` 时，只有
   通过该请求策略的 sealed source 会被本机解密；远端 `env run` 不读取或写入 workspace cache，
   并只以本机已验证 source 对应的内存映射启动 `bun` 子进程。
6. 用户取消确认、PIN/touch 失败、解密失败、协议版本不匹配、SSH 中断和目标进程异常退出均
   不启动目标程序，不留下可复用授权或明文文件。
7. 两个并发 `shine ssh` 会话的策略、令牌、nonce 和请求不能交叉使用；退出任一会话不影响
   另一会话。
8. 不启用该能力时，普通 `shine ssh`、`--with`、`--with-secret`、`shine local` 文件传输及
   Windows remote shell 行为均无回归。
9. 按命令 broker 缺少 `KEY_SECRET` 时拒绝而不回退明文，且始终要求本机逐次确认。一个 SSH
   会话可依次匹配两个不同项目策略，但任何请求都不能复用另一请求的 lease 或 nonce。
10. 相同 source bytes 的完整 `declared_secrets` 必须一致；两个 allow 可用不同 `release`
    子集，本机验证完整 payload key 集后只返回各自子集，未发布的值不出现在响应或子进程环境。
11. ANSI/换行/NUL、超长 argv、过多 source 和超限 source bytes 在本机显示或解密前被拒绝；
    合法远端字段始终以转义、有限长度形式出现在确认 UI。
12. 交互式 SSH 确认前后 termios、echo、Ctrl-C 与窗口 resize 正常，取消和 pinentry 失败均恢复
    SSH/TTY；无 TTY 的 direct request 拒绝，非交互式项目请求仅在显式
    `--trust-remote-session` 时可执行。
13. 策略库 symlink、错误 owner 或过宽 Unix 权限均拒绝加载/更新；`add/update/remove` 原子且
    不受项目配置、overlay 或远端路径影响。
14. `--allow-secret` 在会话中途修改本机配置后仍只解密会话启动时冻结的 ciphertext；下一次
    新 SSH 会话才读取更新值。

## 8. 后续候选能力

- 本机策略映射的稳定 secret reference，避免远端知道本机实际 `[env]` key 名。
- 对支持 token exchange 的服务集成 OIDC/短期凭证，优先返回时效受限凭证而非长期 secret。
- 显式 stdin/匿名 FD 传递接口及应用消费契约。
- Windows 远端的专用受限控制通道与 `env run` 实现。
- 远端可验证工作负载身份或远程度量；在没有这类证明前，不提供“自动批准可抵抗远端失陷”
  的安全承诺。
