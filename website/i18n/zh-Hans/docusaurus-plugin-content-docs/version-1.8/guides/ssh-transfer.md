---
title: SSH 会话：密钥代理与文件传输
sidebar_position: 6
---

# SSH 会话：密钥代理与文件传输

`shine ssh` 会打开一个普通 SSH 会话，可在会话中转发选定环境变量、让远端命令按需请求
本机解密密钥，并附带本机与远端之间的文件传输通道。环境变量与密钥的本机管理方式见
[管理环境变量与密钥](./environment.md#向远端命令提供变量与密钥)。

## 开启会话

```bash
shine ssh user@example.com
shine ssh -p 2222 user@example.com
shine ssh user@example.com uname -a
```

`shine ssh` 会把你提供的 SSH 参数传给系统 `ssh`。进入远端 shell 后，Shine 会设置本次会话需要的环境变量；传输命令必须在这个远端 shell 中运行。

默认的 `posix` 模式要求远端是 macOS 或 Linux，并能运行同版本兼容的 `shine local`。
Windows 可以作为本机端发起这种传输会话。Windows 作为远端时需改用下文的
`--remote-shell windows` 模式，该模式只转发环境，不提供文件传输。

## 转发选定的环境变量

只在本次远端会话或命令中提供本机 Shine 环境值时，把选项写在 SSH 目标之前：

```bash
shine ssh --with API_URL dev
shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'
shine ssh --with-secret API_TOKEN dev
```

`--with` 只读取完全同名的明文 `[env]` 键，不会自动解密 `KEY_SECRET`；需要解密时必须显式使用 `--with-secret KEY[=ALIAS]`。值只写入远端进程环境，不写远端配置文件，但远端 shell 启动文件仍可能再次覆盖同名变量。

转发密钥意味着远端主机可以读取明文；本机或远端具有足够权限、或同一用户的其它进程也可能从进程参数或环境中看到它。只向可信主机转发必要的键，不要把令牌直接写在命令行中。

## 按需向远端命令提供密钥

若远端项目保存的是已封存的 workspace 密文，而私钥或 YubiKey 只留在本机，请用 secret broker。它不会把明文放进登录 shell：远端仅在运行一个已明确允许的子命令时，请求本机解密并注入该子进程。

这个功能只支持 POSIX 远端，且不能保护已被管理员、root 或同账号恶意进程控制的远端。目标程序运行期间仍能读取明文；对可改造的生产服务，优先采用短期凭据或工作负载身份。

### 参数一览

所有以下参数都必须写在 SSH target 前。除 `--secret-broker-inspect` 和
`--secret-broker-enroll` 外，它们以 `--secret-broker` 开启服务会话为前提。

| 参数 | 用途 | 可否重复/组合 |
| --- | --- | --- |
| `--secret-broker` | 开启本次会话的按需密钥代理；远端才可用 `env run --secret-broker` 发起请求 | 可与 `--allow-secret`、`--secret-broker-policy`、`--trust-remote-session` 组合 |
| `--allow-secret KEY[=ALIAS]` | 仅允许远端直接请求一个本机 `[env]` 中的 `KEY_SECRET` 密文 | 可重复；必须搭配 `--secret-broker`，逐次本机确认 |
| `--secret-broker-policy FILE` | 额外加载一份仅本机使用的策略文件 | 可重复；必须搭配 `--secret-broker`，文件仍要通过所有者、权限和非 symlink 检查 |
| `--trust-remote-session` | 使精确匹配 workspace 策略的请求免除逐次确认 | 必须搭配 `--secret-broker`；不影响 `--allow-secret` 的直接请求 |
| `--secret-broker-inspect` | 仅查看远端报告的 workspace、环境源摘要和命令，并与本机策略比较 | 不能与 `--secret-broker` 或 enroll 组合；不释放密钥、不写策略 |
| `--secret-broker-enroll` | 根据远端报告创建本机策略 | 必须同时传 `--trust-remote-metadata`；不能与 broker 或 inspect 组合，不释放密钥、不执行目标命令 |
| `--trust-remote-metadata` | 明确确认“远端本次报告可以作为策略真源” | 仅可与 `--secret-broker-enroll` 一同使用 |
| `--update-policy NAME` | 用受信任的远端报告更新一个已有策略 | 仅可与 enroll 组合；目标、mode 与完整命令必须符合下文的更新约束 |

`--remote-shell windows` 与上述 Secret Broker 参数不能组合，因为 Windows 远端没有所需的 POSIX 控制通道。

临时执行单一命令时，在本机允许指定的加密配置键：

```bash
# 本机：开启一个普通 POSIX SSH 会话；每次请求都需要本机确认。
shine ssh --secret-broker --allow-secret API_TOKEN dev

# 远端：只向下面这个子进程请求 API_TOKEN。
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

`--allow-secret` 与远端的 `--secret` 必须一一对应，均可写成 `KEY=ALIAS`。这种直接请求只读取本机的 `KEY_SECRET` 密文，找不到时会拒绝，绝不退回明文 `KEY`；每次请求都必须在本机确认，`--trust-remote-session` 对它无效。

不要把 `_SECRET` 后缀写给 `--allow-secret`，例如 `--allow-secret API_TOKEN_SECRET` 会被拒绝；应写基础键名 `API_TOKEN`。本机配置即使在会话期间被修改，已允许的直接请求仍使用连接建立时冻结的密文快照。

对于固定项目，先从**本机可信 checkout**登记精确策略。策略记录 SSH target、workspace 和环境源摘要、mode、完整 argv 与可释放的键；不会保存明文：

```bash
shine env broker policy add \
  --name dev-api-build \
  --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development \
  --release DEPLOY_TOKEN \
  -- bun run build

shine ssh --secret-broker dev
```

这里的 `--release DEPLOY_TOKEN` 是“此策略允许本机向目标子进程释放哪一个 workspace
secret”的白名单。它不是把密钥传给远端，也不是选择远端的环境变量：Shine 先验证远端
workspace 与环境源密文的摘要、mode 和完整命令匹配该策略，随后才在本机解密 `DEPLOY_TOKEN`，并且只把它短时注入 `bun run build` 这个子进程。需要多个值时重复写 `--release`；未列出的
`[secret]` 键即使存在于同一环境源中也不会被释放。

如需允许当前 mode 所选环境源里声明的全部 secret，可把重复的 `--release` 换成
`--release-all-declared`。这个选项会在创建描述或策略时展开并记录当时的完整键列表，不是运行时通配符；环境源后来增加 secret 后，旧策略不会自动放行它。两种 release 选择不能同时使用，且至少要选择一种：

```bash
shine env broker policy add \
  --name dev-api-build \
  --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development \
  --release-all-declared \
  -- bun run build
```

远端在该项目目录运行匹配的命令：

```bash
shine env run --mode development --secret-broker -- bun run build
```

默认仍会逐次在本机确认。只有已理解“信任该远端 SSH 会话及同账号进程”这一风险时，才在本机 SSH 命令上添加 `--trust-remote-session`；它只会自动批准精确匹配的 workspace 策略。

如果策略不应写进默认的 `~/.shine/ssh-secret-broker.toml`，可额外加载受本机保护的临时或团队策略文件。它与默认策略合并；请求必须恰好命中一条策略，零条或多条都会拒绝：

```bash
shine ssh --secret-broker \
  --secret-broker-policy ~/.config/shine/staging-broker.toml \
  staging
```

策略变化前可检查差异，之后显式更新；命令参数不是通配匹配，变更 build 命令、mode 或环境源都需要重新审阅：

```bash
shine env broker policy diff dev-api-build \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development --release DEPLOY_TOKEN -- bun run build
shine env broker policy update \
  --name dev-api-build --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development --release DEPLOY_TOKEN -- bun run build
```

若没有本机 checkout，可使用 inspect 会话查看候选描述。先在本机开启 inspect，再在远端项目目录运行 `describe`；该请求会显示 workspace、source 摘要、mode、完整 argv 与 release 键，并与本机策略比较：

```bash
# 本机
shine ssh --secret-broker-inspect dev

# 远端
cd /srv/acme-api
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

`describe` 中的 `--release` 同样是候选策略的可释放键名单。inspect 只显示并比较该名单；enroll 在本机确认后将它写入本机策略，二者都不会解密或传输这些值。

inspect 不释放密钥，也不会创建或修改策略。若已通过带外方式确认远端主机和这份元数据的完整性，可改用 enroll；`--trust-remote-metadata` 是强制的显式确认：

```bash
# 本机
shine ssh --secret-broker-enroll --trust-remote-metadata dev

# 远端：仍然只发送描述，不会执行 build 或解密。
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

enroll 会在本机显示候选内容并要求本地确认后才写入策略；同名或重叠策略不会静默覆盖，应回到可信 checkout 使用 `policy diff` / `policy update` 审阅变更。

只有远端副本可用、且仍愿意把它报告的元数据作为真源时，才可显式更新已有策略：

```bash
# 本机：指定要更新的已有策略。
shine ssh --secret-broker-enroll --trust-remote-metadata \
  --update-policy dev-api-build dev

# 远端：mode 与完整命令必须命中该策略中恰好一条 allow。
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

Shine 还会要求该策略属于当前 SSH target；若策略限定了远端 workspace 路径，本次报告也必须一致。确认界面会显示完整 TOML diff；批准后只替换命中的 allow 并刷新 workspace/source 摘要，保留策略名、project、远端路径约束及其它 allow。预览后策略若被其它进程修改，写入会失败，需重新检查。`--update-policy` 不会降低远端元数据的信任风险，优先使用可信本机 checkout 的 `policy diff` / `policy update`。

## 连接 Windows 远端

Windows OpenSSH 远端需要在 SSH 目标之前显式选择 PowerShell 包装器：

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN windows-host
shine ssh --remote-shell windows --with API_URL windows-host Get-ChildItem Env:API_URL
```

Shine 会优先使用远端的 PowerShell 7（`pwsh.exe`），未安装时回退到 Windows PowerShell
5.1（`powershell.exe`），并安全注入所选环境变量和本机终端主题。交互式会话会加载所选
PowerShell 的正常 profile，因此 Shine 管理的 `PATH` 和 source-command wrapper 可以生效；
显式远端命令以 no-profile 模式执行。

该模式不会建立传输隧道，因此不能在远端运行 `shine local download`、`upload` 或
`status`。如需与 Windows 远端传文件，请直接使用系统 `scp`、`sftp` 或其它传输工具。

## 从远端下载到本机

在 `shine ssh` 打开的远端 shell 中运行：

```bash
shine local download ./logs/app.log
shine local download ./logs/app.log ./downloaded/app.log --dry-run
shine local download ./logs/app.log ./downloaded/app.log --force
shine local download ./logs/app.log --scp
shine local download ./dist ./dist-copy
```

`download` 的来源路径由远端解析，目标路径由本机解析。未指定目标时，Shine 会把文件或目录放到本机启动 `shine ssh` 时所在目录，并沿用来源名称。

Shine 会在本机端重新发起系统 `rsync` 或 `scp` 传输；优先使用 `rsync`，不可用时回退到 `scp`。两端都需要可用的 `ssh`，目录传输还需要两端具有同一种可用工具（优先 `rsync`，否则 `scp`）。目标文件已存在时默认拒绝覆盖；目录已存在时加 `--force` 表示合并写入。
需要跳过 `rsync` 探测并强制使用 `scp` 时，添加 `--scp`。

## 从本机上传到远端

仍然在远端 shell 中运行：

```bash
shine local upload ./release.tar.gz /tmp/release.tar.gz --dry-run
shine local upload ./release.tar.gz /tmp/release.tar.gz --force
shine local upload ./site /tmp/site
```

`upload` 的来源路径由本机解析，目标路径由远端解析。未指定目标时，Shine 会把文件或目录放到远端当前目录，并沿用来源名称。

上传目录时，Shine 会拒绝把文件覆盖成目录或把目录覆盖成文件。先运行 `--dry-run` 可以确认解析后的两端路径和覆盖结果。

## 查看连接状态

```bash
shine local status
```

状态输出会显示会话 ID、连接是否可达、协议版本，以及本机端的默认目录。若当前 shell 不是通过 `shine ssh` 进入的，`shine local` 会提示缺少会话环境变量。
