---
title: SSH 会话：密钥代理与文件传输
sidebar_position: 6
---

# SSH 会话：密钥代理与文件传输

`shine ssh` 会打开一个普通 SSH 会话，可在会话中转发选定环境变量、让远端命令按需请求
本机解密密钥，并附带本机与远端之间的文件传输通道。环境变量与密钥的本机管理方式见
[管理环境变量与密钥](./environment.md#向远端命令提供变量与密钥)。

## 版本兼容性 {#version-compatibility}

Shine 通道报告协议版本不匹配时，升级本地或远端较旧的 Shine，再重新连接。
用于自动化流程时，请提前协调两端的升级。

## 开启会话

按需选择一种连接方式：

| 操作 | 命令 |
| --- | --- |
| 打开交互远端 shell | `shine ssh user@example.com` |
| 指定 SSH 端口 | `shine ssh -p 2222 user@example.com` |
| 连接 IPv6 地址 | `shine ssh user@2001:db8::1` |
| 运行一条远端命令 | `shine ssh user@example.com uname -a` |

`shine ssh` 会把你提供的 SSH 参数传给系统 `ssh`。文件传输命令必须在 Shine 打开的远端 Shell 中运行。

默认的 `posix` 模式要求远端是 macOS 或 Linux，并能运行同版本兼容的 `shine local`。
Windows 可以作为本机端发起这种传输会话。Windows 作为远端时需改用下文的
`--remote-shell windows` 模式，该模式只转发环境，不提供文件传输。

传输中断时，已经复制的文件不会自动回滚。重试前请检查两端文件。

## 转发选定的环境变量

只在本次远端会话或命令中提供本机 Shine 环境值时，把选项写在 SSH 目标之前：

| 操作 | 命令 |
| --- | --- |
| 转发普通变量 | `shine ssh --with API_URL dev` |
| 重命名转发变量 | `shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'` |
| 转发解密后的密钥 | `shine ssh --with-secret API_TOKEN dev` |

`--with` 只读取完全同名的明文 `[env]` 键，不会自动解密 `KEY_SECRET`；需要解密时必须显式使用 `--with-secret KEY[=ALIAS]`。值只写入远端进程环境，不写远端配置文件，但远端 shell 启动文件仍可能再次覆盖同名变量。

转发密钥意味着远端主机可以读取明文；本机或远端具有足够权限、或同一用户的其它进程也可能从进程参数或环境中看到它。只向可信主机转发必要的键，不要把令牌直接写在命令行中。

## 按需向远端命令提供密钥

若远端项目保存的是已封存的 workspace 密文，而私钥或 YubiKey 只留在本机，请用 secret broker。它不会把明文放进登录 shell：远端仅在运行一个已明确允许的子命令时，请求本机解密并注入该子进程。

这个功能只支持 POSIX 远端，且不能保护已被管理员、root 或同账号恶意进程控制的远端。目标程序运行期间仍能读取明文；对可改造的生产服务，优先采用短期凭据或工作负载身份。

### 参数一览

Shine 参数都放在 SSH 目标前。需要释放密钥时，使用 `--secret-broker` 开启会话；检查模式
（`--secret-broker-inspect`）和录入模式（`--secret-broker-enroll`）使用独立会话，三者不能组合。
所有 broker 模式都要求 POSIX 远端，不能与 `--remote-shell windows` 同用。

下文按任务给出示例，完整选项见 `shine ssh --help`。

### 为单条命令使用本机密钥

临时执行单一命令时，在本机允许指定的加密配置键：

```bash
# 本机：开启一个普通 POSIX SSH 会话；每次请求都需要本机确认。
shine ssh --secret-broker --allow-secret API_TOKEN dev

# 远端：只向下面这个子进程请求 API_TOKEN。
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

`--allow-secret` 与远端的 `--secret` 必须一一对应，均可写成 `KEY=ALIAS`。这种直接请求只读取本机的 `KEY_SECRET` 密文，找不到时会拒绝，绝不退回明文 `KEY`；每次请求都必须在本机确认，`--trust-remote-session` 对它无效。

传入基础键名 `API_TOKEN`，不要带 `_SECRET` 后缀。本机密钥修改后请重新连接；已有会话继续使用
建立连接时选定的值。

### 授权项目命令

对于固定项目，先从**本机可信的项目副本**创建策略，允许所选密钥用于指定 SSH 目标、工作区版本、
模式和完整命令。策略不会保存明文：

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

`--release DEPLOY_TOKEN` 允许该工作区密钥提供给获批命令；创建策略时不会发送它。
多个键可以重复使用 `--release`，未列出的密钥不能释放。

也可改用 `--release-all-declared`，允许当前模式声明的全部密钥。两种方式不能组合，必须选择一种；
它只覆盖当前的键，不会自动允许后来新增的密钥。

远端在该项目目录运行匹配的命令：

```bash
shine env run --mode development --secret-broker -- bun run build
```

Workspace 值与本地 `env run` 一样按环境源顺序覆盖。后面的明文值会覆盖前面的 secret，被覆盖的
secret 不会发送到远端。后面的 secret 声明会隐藏前面的明文值，即使该 secret 没有获准释放。

默认仍会逐次在本机确认。只有已理解“信任该远端 SSH 会话及同账号进程”这一风险时，才在本机 SSH 命令上添加 `--trust-remote-session`；它只会自动批准精确匹配的 workspace 策略。

### 加载或更新策略

除默认的 `~/.shine/ssh-secret-broker.toml` 外，也可加载临时或团队策略文件：

```bash
shine ssh --secret-broker \
  --secret-broker-policy ~/.config/shine/staging-broker.toml \
  staging
```

策略文件须留在本机并保持私有。Unix 上文件须归当前用户所有，权限为 `0600`，且不能是符号链接。
可重复使用 `--secret-broker-policy` 加载多份文件；请求必须在全部文件中恰好匹配一条策略，
无匹配或多条匹配都会被拒绝。

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

添加已有名称的策略会失败，不会覆盖原策略。更新时应使用 `--name` 并重新提供策略参数，
包括原有的 project 或远端 workspace 限制；详见 `shine env broker policy update --help`。
策略发生变化时仍需审阅并确认。

使用 `shine env broker policy list` 查看策略列表，用 `shine env broker policy info <NAME>`
查看某项策略。审阅期间策略发生变化时，请检查新版本后重试。

### 审阅仅存在于远端的项目

没有可信本机副本时，先检查远端项目再创建策略。在本机开启 inspect 会话，在远端项目中运行
`describe`，审阅它的工作区、源文件、模式、命令及拟申请的密钥权限，并与本机策略比较：

```bash
# 本机
shine ssh --secret-broker-inspect dev

# 远端
cd /srv/acme-api
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

检查不会释放密钥或修改策略，`describe` 中的 `--release` 只说明拟申请的权限。
独立确认远端主机和项目可信后，重新以录入模式连接并创建策略：

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

策略必须匹配当前 SSH 目标及限定的工作区路径。确认前请在本机审阅拟议变更。
从远端元数据更新策略仍需信任该远端；有已验证的本机 checkout 时，应优先使用它。

### 通过 broker 读取混合秘密

使用混合秘密时，在 broker 所在本机的全局配置中设置 `hybrid_decrypt_backend`。远端不能选择
解密后端或身份。除非本次会话已允许精确匹配策略的请求免确认，否则仍需本机批准；取消后不会
尝试其他后端。远端只收到获准的明文值，不会收到解密密钥。重新封存或改变收件人后，请重新
审阅并更新受影响的策略。

## 在本机授权秘密文件 {#file-secret-broker}

文件 broker 可在本机解密文件，供 POSIX 远端执行获批操作。使用前请升级两端 Shine；版本不匹配
会阻止释放，已有的环境变量策略也不会授权文件访问。

对于开启自动解密的 App，可从可信的本地 overlay 创建精确策略：

```bash
shine env broker policy add --name service-install --ssh-target server \
  --workspace ~/overlay/app/example/shine.workspace.toml \
  --remote-workspace /srv/example-service/shine.workspace.toml \
  --mode production --file-group credentials --app example --operation install \
  --migrate-file-policy
shine ssh --secret-broker server
# 服务器上执行：
shine app install example
```

升级需要另建 `--operation upgrade` 策略。使用 `--migrate-file-policy` 前，请私下备份
`~/.shine/ssh-secret-broker.toml`：该选项会升级策略格式并保留已有环境策略。降级 Shine 不会
撤销这项迁移。

请审阅获准的主机、操作、文件、目标路径和权限。请求只能使用获准文件；新增或修改来源后需重新
审阅并更新策略。`policy update`、`policy diff` 接受相同的文件参数。

没有可信本地检出时，可使用已有的独立检查／录入会话：

```bash
shine ssh --secret-broker-inspect server
# 服务器上检查，不解密：
shine env broker describe --app example --operation install \
  --mode production --file-group credentials
```

只有信任远端描述时才使用 `shine ssh --secret-broker-enroll --trust-remote-metadata server`。
在远端运行相同的 describe 命令并在本机确认，然后重新以 `--secret-broker` 连接。
`--update-policy NAME` 用于指定要替换的已有文件策略。检查和录入会话从不返回明文。

手动文件导出应省略 `--app`，为安装后的 workspace 单独授权 export 操作，然后在远端执行：

```bash
shine env workspace export --format files --workspace /srv/example-service/shine.workspace.toml \
  --mode production --file-group credentials --secret-broker
```

默认仍需本机确认。`--trust-remote-session` 只能在精确策略匹配后跳过确认；
私钥留在本机，但获准的明文会传到远端。手动导出拒绝覆盖已有目标，不会将文件纳入 App 管理；
其权限和清理仍由你负责。

## 连接 Windows 远端

Windows OpenSSH 远端需要在 SSH 目标之前显式选择 PowerShell 包装器：

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN windows-host
shine ssh --remote-shell windows --with API_URL windows-host Get-ChildItem Env:API_URL
```

Shine 优先使用 PowerShell 7，否则使用 Windows PowerShell 5.1。交互会话会加载正常的
PowerShell profile，显式远端命令不会。如果命令依赖 profile 中的设置，远端执行时需考虑这一差异。

该模式不会建立传输隧道，因此不能在远端运行 `shine local download`、`upload` 或
`status`。如需与 Windows 远端传文件，请直接使用系统 `scp`、`sftp` 或其它传输工具。

## 从远端下载到本机

在 `shine ssh` 打开的远端 Shell 中运行以下命令。允许覆盖前先预览目标；SCP 即使不带
`--force` 也可能覆盖已有文件。

| 操作 | 命令 |
| --- | --- |
| 下载到本地连接起始目录 | `shine local download ./logs/app.log` |
| 预览目标路径 | `shine local download ./logs/app.log ./downloaded/app.log --dry-run` |
| 允许覆盖目标文件 | `shine local download ./logs/app.log ./downloaded/app.log --force` |
| 直接使用 SCP | `shine local download ./logs/app.log --scp` |
| 下载目录 | `shine local download ./dist ./dist-copy` |

`download` 的来源路径由远端解析，目标路径由本机解析。未指定目标时，Shine 会把文件或目录放到本机启动 `shine ssh` 时所在目录，并沿用来源名称。

Shine 会在本机端重新发起系统 `rsync` 或 `scp` 传输；优先使用 `rsync`，不可用时回退到 `scp`。两端都需要可用的 `ssh`，目录传输还需要两端具有同一种可用工具（优先 `rsync`，否则 `scp`）。Rsync 默认跳过已有文件，指定 `--force` 才会覆盖；目录已存在时加 `--force` 表示合并写入。
需要跳过 `rsync` 探测并强制使用 `scp` 时，添加 `--scp`。
SSH 目标直接使用 IPv6 地址时，为兼容系统 rsync，传输会使用 SCP。SCP 即使未指定 `--force`
也可能覆盖已有文件，Shine 会显示警告；其 `--dry-run` 只显示解析后的路径，不复制文件。
使用 rsync 时，在输入的命令中为含空格的路径和通配模式加引号：

```bash
shine local download './build output/*.log'
shine local upload './release notes.txt' '/tmp/release files'
```

空格和 shell 标点会保留为远端路径的一部分；`*`、`?` 和方括号模式仍可匹配远端文件。

## 从本机上传到远端

仍在同一远端 Shell 中运行上传命令。先预览，只有明确需要覆盖时才使用 `--force`。

| 操作 | 命令 |
| --- | --- |
| 预览上传 | `shine local upload ./release.tar.gz /tmp/release.tar.gz --dry-run` |
| 允许覆盖目标文件 | `shine local upload ./release.tar.gz /tmp/release.tar.gz --force` |
| 上传目录 | `shine local upload ./site /tmp/site` |

`upload` 的来源路径由本机解析，目标路径由远端解析。未指定目标时，Shine 会把文件或目录放到远端当前目录，并沿用来源名称。

上传目录时，Shine 会拒绝把文件覆盖成目录或把目录覆盖成文件。先运行 `--dry-run` 可以确认解析后的两端路径和覆盖结果。

本机传输路径必须是有效 UTF-8。本机路径提示只展开开头的 `~`；`$VAR` 和 `${VAR}`
保留为文件名文本。

## 查看连接状态

```bash
shine local status
```

用结果检查连接是否可达，并确认本机默认目录。若提示缺少会话变量，请用 `shine ssh` 重新连接。
连接或认证失败时，请检查 SSH 会话后再重试传输。
