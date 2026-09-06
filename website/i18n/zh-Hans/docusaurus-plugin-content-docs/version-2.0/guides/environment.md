---
title: 管理环境变量与密钥
sidebar_position: 5
---

# 管理环境变量与密钥

Shine 可以保存预设模板变量，也可以使用 GPG 或 age 封存项目环境中的敏感值。封存后的
密钥既可以提供给本机子进程，也可以通过 SSH Secret Broker 由远端命令按需请求本机解密。
不要把真实密钥写进公开仓库或文档示例。

密钥操作统一位于 `shine env secret` 下；workspace 形式的 `shine env run` 和 `env run --with`
用于按需向子进程注入变量。若目标命令在 SSH 远端，先阅读下文“向远端命令提供变量与密钥”，
选择直接转发或按需解密。

## 查看和设置变量

```bash
shine env list
shine env get HTTP_PROXY_PORT
shine env set HTTP_PROXY_PORT 6152
shine env delete HTTP_PROXY_PORT
```

`PROXY_NO_PROXY` 控制 `setproxy` 设置的 `NO_PROXY` 和 `no_proxy`，默认为
`localhost,127.0.0.1,::1`。修改它或其他代理变量后，`shine update` 会把已安装的
`proxy` shell 预设标记为可更新；运行 `shine upgrade` 应用新值。

内置图片命令默认使用 `IMAGE_QUALITY=80`、`IMAGE_MAX_WIDTH=1920`、
`IMAGE_MAX_HEIGHT=1080`。可用 `--quality`、`--width`、`--height` 只覆盖当次运行，也可用
`shine env set` 为当前机器保留不同默认值。

`shine env list` 默认隐藏敏感值；`--reveal` 会显示完整值，应只在安全终端中使用。输出会按实际来源分为 `config.toml`、全局覆盖文件、overlay 和项目覆盖文件，便于确认哪个值正在生效。变量通常保存到当前配置的 `[env]` 表。

全局 `~/.shine/config.toml` 和项目 `shine.config.toml` 的 `[env]` 支持简写字符串，也支持
同时记录值和说明：

```toml
[env]
HTTP_PROXY_PORT = "6152"
MY_API_TOKEN = { value = "<令牌>", description = "内部 API 的访问令牌" }
```

`value` 的使用方式与简写字符串完全相同；`description` 会显示在 `shine env list` 中。
对已有详细条目执行 `shine env set MY_API_TOKEN <新值>` 时，Shine 会更新 `value` 并保留
说明。

若同名键已由全局、overlay 或项目 `shine.env.toml` 覆盖，直接 `set`、`delete` 或 `env secret encrypt --set` 会被拒绝，防止写入一个不会生效的低优先级值。确认应修改该覆盖文件时，添加 `--force`：

```bash
shine env set HTTP_PROXY_PORT 7890 --force
shine env delete HTTP_PROXY_PORT --force
shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET --force
```

对于 `shine preset overlay link --git` 管理的镜像，`--force` 写入会在下次 `shine preset pull` 时被丢弃；应改在 overlay 上游仓库维护该值。

不带 `[env]` 表头的全局、overlay 和项目 `shine.env.toml` 覆盖文件也支持这两种格式：

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "本地代理主机" }
```

详细项同时覆盖值和说明；字符串只覆盖值，并保留低优先级配置或 preset catalog 提供的
说明。数字、数组、缺少 `value` 等无效条目会直接报错，不会被静默忽略。

修改用于模板渲染的值后，运行：

```bash
shine upgrade
```

## 使用 GPG 加密值

先确认本机的 `gpg` 可以使用对应公钥；私钥保存在 YubiKey 时，可参考
[在 macOS 和 Windows 使用 YubiKey OpenPGP](https://blog.biulight.top/timeline/knowledge/yubikey-openpgp)完成接入。然后在
`~/.shine/config.toml` 中指定默认接收者（可同时加密给多把 GPG 公钥）：

```toml
gpg_recipients = ["user@example.com", "team-backup@example.com"]
```

将已有明文变量加密并保存为另一个 key：

```bash
shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET
shine env secret decrypt MY_TOKEN_SECRET
```

加密只需要接收者公钥；解密时才需要连接持有对应私钥的 YubiKey，并按提示输入 PIN 或触摸设备。

旧版的单值 `gpg_key_id` 已废弃。先用 `shine state migrate --dry-run` 查看，再运行
`shine state migrate` 将它转换为 `gpg_recipients`；工作区中旧的
`[env.encryption].recipient` 也会在 `env run` 或 `env secret seal` 需要使用时提示迁移。

需要导出到当前 shell 时：

```bash
eval "$(shine env secret export MY_TOKEN)"
eval "$(shine env secret export MY_TOKEN --as API_TOKEN)"
```

安装 `utils` shell 预设后，也可以使用 `shine-env-export MY_TOKEN --as API_TOKEN`。

## 使用 age identity

Shine 支持 `age` 作为第二种密钥后端。它适合把密文提交到团队仓库中，并加密给多个成员各自的 recipient。已有 GPG 密文不需要迁移：不带标签的旧密文继续按 GPG 解密，`age` 后端生成的新密文会带有 `age:` 标签。

先确认标准 `age` CLI 已安装并位于 `PATH` 中。生成一个普通软件 identity，并记录输出中的
recipient：

```bash
shine env secret identity init
shine env secret identity list
```

普通 identity 使用 `age-keygen`，默认写入 `~/.shine/age/identity.txt`。

### 在 macOS 上使用 Touch ID

如果希望每次解密都需要本机用户授权，macOS 上可以改用由 Secure Enclave 托管的 identity。
这种方式还需要安装 `age-plugin-se`；Homebrew 可以同时安装两个依赖：

```bash
brew install age age-plugin-se
```

不要再执行上面的普通 identity 初始化，而是运行 Touch ID 形式，然后记录它的 recipient：

```bash
shine env secret identity init --touch-id
shine env secret identity list
```

`--touch-id` 只适用于 macOS。解密时需要本机 Secure Enclave，并会触发 Touch ID 或系统密码
授权；即使 identity 文件被复制到另一台机器，通常也不能直接解密。看到意外的授权提示时，
应取消授权并检查触发它的命令。如果 AI Agent 可以在本机运行命令，请继续阅读
[在 AI Agent 参与开发时保护环境密钥](./agent-secret-safety.md#touch-id-改善什么)，了解这种方式的安全边界。

如需在本机所有项目中使用同一默认后端和 recipient，将它们写入 `~/.shine/config.toml`：

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]
```

旧的 `age_identity` 单路径会与新增的 `age_identities` 路径列表按顺序合并并去重。这样普通
identity、Secure Enclave identity 和硬件 plugin stub 可以共存，不需要复制任何文件。

如果 recipient 是某个项目团队共享的名单，应将它写入项目根目录的
`shine.workspace.toml` 的 `[env.encryption]`；这样可以随项目提交，而不会影响本机的其他
项目。该配置会优先于全局默认值，完整格式见下文“使用分层项目环境”。不要将私有
`age_identity` 提交到仓库。

也可以只在单次命令中选择后端和 recipient：

```bash
shine env secret encrypt --backend age -r age1se1qexample... -r age1qteammate... --from MY_TOKEN
shine env secret seal --backend age -r age1se1qexample... -r age1qteammate...
```

`-r/--recipient` 对 GPG 和 age 都可以重复使用。移除某个 recipient 不会撤销它对历史密文的访问；需要重新加密或重新 `seal` 才能轮换访问范围。

### 在 Windows 上实验手机授权

[`age-plugin-phone`](https://github.com/biulight/age-plugin-phone) 目前仍是 owner-only 技术预览，只能用于合成或可丢弃数据，不能保护真实或生产 secret。当前 Windows Alpha 要求 Windows 11 x64 客户端、TPM 2.0、Microsoft Platform Crypto Provider，以及能力检查合格的 Android StrongBox 手机。制品校验、配对、传输、恢复演练和清理步骤以项目的 [`Windows Alpha quick start`](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-alpha-quickstart.md) 为准。

安装同一版本的桌面 plugin 和 Android 应用后，可以通过 Shine 启动 plugin 自己的事务式配对：

```powershell
shine env secret identity init --phone --label "NUC WiFi Pair" --transport auto
```

如果希望通过局域网完成配对，先在手机端打开显式的 **Pair · Wi-Fi** 操作，再运行上述
命令。Windows 上的 `auto` 会先执行一次有界的 Wi-Fi discovery：只有一个匹配且位于前台的手机
listener 响应时选择 Wi-Fi；没有 listener 响应时，会在创建 pairing offer 之前选择
Developer USB/ADB。多个响应或本机 discovery 错误会安全失败；协议处理开始后不会再切换
transport。`auto` 是默认值，因此省略 `--transport auto` 时策略不变。

Developer USB 的顺序相反：先启动桌面命令；plugin 选择 ADB 并开始等待手机连接后，再在手机端
点击 **Pair · USB**。使用 `--transport adb` 时，plugin 会在预检完成后直接选择 ADB。手机只会
立即尝试连接一次，因此如果在桌面建立 `adb reverse` 规则前点击 **Pair · USB**，手机会报告
`usb_transport_failed`。

配对标签默认使用 Windows 计算机名；也可以显式指定标签、固定使用 Developer USB 或 QR，
以及在存在多台 ADB 设备时指定序列号：

```powershell
shine env secret identity init --phone --label "Work laptop"
shine env secret identity init --phone --transport adb
shine env secret identity init --phone --transport qr
shine env secret identity init --phone --adb-serial SERIAL
```

配对、TPM、replay、locator、中断恢复和清理状态仍完全由 plugin 管理。完整指纹确认成功后，
Shine 只会把公开 identity stub 路径加入当前用户的全局 `age_identities`。如果当前项目显式
覆盖了 `age_identity` 或 `age_identities`，命令会在配对前退出，避免创建一个随后被项目忽略
的 identity。对应的手工配置形式如下：

```toml
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]
```

这个快捷命令不会修改 `secret_backend`，也不会自动添加 recipient。plugin setup 如果中断，
必须按照其文档使用 `age-plugin-phone setup --resume` 或 `age-plugin-phone setup --cleanup`；
不要把重新发起一次配对当作恢复手段。

在可提交到仓库的项目 `shine.workspace.toml` 中，同时加入配对输出的 `age1phone...` recipient 和一个已经独立验证过的恢复 recipient：

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1phone...", "age1..."]
```

封存只使用 recipient 的公开材料，不会在手机上弹出授权提示。使用 `auto` 配对后，如果希望后续
解密也优先走 Wi-Fi，请开启 **Wi-Fi auto-listen** 并保持手机应用在前台。plugin 会在创建
unwrap request 前发现匹配的 listener；未发现时在 Windows 上选择 Developer USB/ADB，不会并行竞速或
在请求开始后自动重试其它路径。解密由手机保护的 secret（包括通过 `shine env run` 使用它）时，
标准 age plugin 会为每次 file key 解包要求一次新的强生物验证。Developer USB 和 Wi-Fi 的 plugin 提示默认
静默；设置 `AGE_PLUGIN_PHONE_MESSAGES=1` 可显式开启。QR 请求必须由手机扫描，因此仍会显示在终端中。
`shine env secret decrypt` 成功时只写入解密值，同时屏蔽 age 客户端自身的等待提示，并且不会额外添加换行。
Shell 主题仍可能主动把下一条 prompt 放到新行。对于需要保留的数据，绝不能只配置这个实验性手机
recipient；恢复路径不能依赖同一部手机的 StrongBox 密钥、同一台 Windows 电脑的 TPM 密钥或该 plugin 的本地状态。

如果 AI Agent 会参与开发，先阅读[在 AI Agent 参与开发时保护环境密钥](./agent-secret-safety.md)，确认 identity 文件、Touch ID、手机授权提示和命令执行权限的安全边界。

## 只向一个命令提供变量

不修改当前终端、也不创建 workspace 文件时，使用可重复的 `--with`：

```bash
shine env run --with MY_TOKEN -- bun run build
shine env run --with MY_TOKEN=API_TOKEN -- bun run build
shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build
shine env run --no-workspace --with MY_TOKEN -- bun run build
```

每个 `KEY` 都优先解密 `<KEY>_SECRET`，不存在时才读取明文 `<KEY>`。等号右侧是子进程中
的变量名。显式 `--with` 值优先于当前进程和 workspace 的同名变量。

`--no-workspace` 会完全跳过 `shine.workspace.toml` 查找，只合并当前进程环境和显式
`--with`；它不能与 `--workspace` 或 `--mode` 同时使用。这个模式也用于需要固定读取
Shine 配置环境、但不应受当前工作目录影响的受管 Bun 命令入口。

## 选择单次注入还是透明代理

偶尔执行一次敏感操作时，优先使用单次注入。例如，Cargo 的 `cargo:token` credential
provider 启用时，可以通过 `CARGO_REGISTRY_TOKEN` 读取 crates.io token，因此执行 yank 时
不必把 token 留在 Shell 中，也不必长期启用命令代理：

```bash
shine env run --no-workspace \
  --with CARGO_REGISTRY_TOKEN \
  -- cargo yank my-crate@1.2.3
```

`--with CARGO_REGISTRY_TOKEN` 会优先解密 `CARGO_REGISTRY_TOKEN_SECRET`，只在本次运行中
注入 Cargo。Cargo 及其启动的后代进程仍然可以读取该值。日常持久使用 Cargo 认证时，Cargo
官方更推荐操作系统 credential provider；只有明确希望把 token 加密保存在 Shine 中时，才选择
Shine 注入。详见 [Cargo registry authentication](https://doc.rust-lang.org/stable/cargo/reference/registry-authentication.html)
和 [`cargo yank`](https://doc.rust-lang.org/stable/cargo/commands/cargo-yank.html)。

### 为固定凭据变量安装透明代理

如果一个 CLI 每次调用都需要同一个固定凭据变量，可以安装透明代理。有些 CLI（例如 GitHub
CLI）不会接受从命令行传入的 token，而是读取 `GH_TOKEN` 这类环境变量：

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

Shine 会在 `~/.shine/bin/` 创建同名 shim，并记录当前 `PATH` 中找到的真实命令。运行 `gh`
时，shim 只在它的子进程中解析 `GH_TOKEN_SECRET`；若不存在密文，才读取明文 `GH_TOKEN`。
该值不会写回或导出到父 Shell。`--with` 可重复使用，也可写成 `KEY=ALIAS`，以不同的变量名
传给目标命令。

代理规则作用于整个命令，不能只匹配某个子命令。如果明确要代理 Cargo，应在不需要 token 时
停止注入：

```bash
shine env proxy install cargo --with CARGO_REGISTRY_TOKEN
shine env proxy disable cargo

# 之后只在需要凭据的操作前启用：
shine env proxy enable cargo
cargo yank my-crate@1.2.3
shine env proxy disable cargo
```

启用期间，每个 Cargo 子命令及其后代进程都可能继承 token。`disable` 会保留 shim，只停止
解密和注入，并直接转发到真实 Cargo。偶尔执行 yank 时，应优先采用上面的单次 `env run`。

只代理你明确允许的裸命令名；命令名只能包含 ASCII 字母、数字、`-`、`_` 或 `.`。安装前请确认
`~/.shine/bin/` 已在 `PATH` 的靠前位置，且目标命令不是另一个 Shine 代理。若同名入口已存在
但并非 Shine 创建，安装会拒绝覆盖它。

默认规则保存在全局 `~/.shine/config.toml`。在含有 `shine.config.toml` 的项目内加入
`--project`，可将该命令的规则限定到项目；同一命令的项目规则会覆盖全局规则：

```bash
shine env proxy install gh --with GH_TOKEN --project
shine env proxy list
```

需要临时保留 shim、但禁止任何解密或注入时，可禁用规则。禁用后命令会直接转发给真实程序：

```bash
shine env proxy disable gh
shine env proxy enable gh
shine env proxy disable gh --project
```

不再需要代理时，移除 Shine 管理的 shim 及其用户级规则：

```bash
shine env proxy uninstall gh
```

如果真实命令被升级、移动或删除，重新执行安装命令以记录新的目标路径。

## 向远端命令提供变量与密钥

通过 `shine ssh` 运行远端命令时，根据远端需要看到明文的范围选择方式：

| 目标 | 本机命令 | 明文可见范围 |
| --- | --- | --- |
| 转发普通变量 | `shine ssh --with API_URL dev` | 远端登录 shell 或指定命令 |
| 解密并直接转发一个密钥 | `shine ssh --with-secret API_TOKEN dev` | 远端登录 shell 或指定命令 |
| 由远端子命令按需请求本机解密 | `shine ssh --secret-broker ... dev` | 仅获准启动的远端子进程 |

`--with-secret KEY[=ALIAS]` 会在建立会话时解密本机 `KEY_SECRET`，适合可信远端上的临时
操作。远端登录 shell 及同账号进程可能读取该明文，不应把它理解为受隔离的密钥通道。

若私钥、age identity 或 YubiKey 只保留在本机，而远端项目保存已封存的 workspace 密文，
使用 SSH Secret Broker。远端只提交待运行命令和密钥请求，本机会校验允许列表或精确策略，
确认后在本机解密，再把明文短时注入获准的远端子进程：

```bash
# 本机：允许远端按需请求 API_TOKEN；每次请求都在本机确认。
shine ssh --secret-broker --allow-secret API_TOKEN dev

# 远端：只向这个子进程注入 API_TOKEN。
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

Secret Broker 不会把解密私钥传到远端，也不会把明文放进远端登录 shell；但目标子进程、
远端管理员和同账号恶意进程仍可能读取明文。固定项目应使用绑定 workspace 摘要、mode、完整
命令和可释放键的本机策略。完整的策略登记、检查、更新与安全边界见
[SSH 会话：密钥代理与文件传输](./ssh-transfer.md#按需向远端命令提供密钥)。

## 从 dotenv 初始化工作区

已有 Vite 风格的 `.env` 文件时，可在项目根目录生成 Shine workspace 和对应的 TOML 环境源：

```bash
shine env workspace init --from-dotenv --dry-run
shine env workspace init --from-dotenv
```

它读取 `.env`、`.env.local`、`.env.<mode>` 与 `.env.<mode>.local`，自动发现 mode，并保持该覆盖顺序。原 dotenv 文件不会被修改；生成目标已存在时命令会拒绝覆盖，确认后才添加 `--force`。只导入指定 mode 时可重复使用 `--mode`：

```bash
shine env workspace init --from-dotenv --mode development --mode production
```

导入时可将明确知道的敏感键放进 `[secret]`，之后配置 recipient 并封存。未标记的值会作为明文导入；不要把实际凭据误当作普通配置提交。

```bash
shine env workspace init --from-dotenv --secret DATABASE_URL
shine env secret seal
```

为避免改变 dotenv 语义，包含插值（例如 `${BASE_URL}`）或带转义的双引号值的文件会被拒绝；先将它们解析为最终值后再导入。即使没有选择 `--secret`，生成文件仍会保留带说明的空 `[secret]` 表。

## 将 workspace 导出为 dotenv

当其他工具需要普通 dotenv 文件，或者准备停止使用 Shine env 时，可导出某个 mode 合并后的最终结果：

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local
```

`--format` 为必填项，以便明确导出格式。命令按 workspace 声明的顺序合并环境源，但不会混入当前进程变量或 `--with` 值。默认只导出最终生效的 `[plain]` 项，不会解密 payload；若后层 secret 覆盖了前层 plain，同名旧明文也不会被导出。

只有目标确实需要完整可运行环境时，才显式包含已经封存的 secret：

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local \
  --include-secrets
```

这会把 secret 以明文写入文件。在 Unix 上，含 secret 的新输出文件权限为仅所有者可读写的 `0600`；无论使用什么平台，都应将它排除在版本控制之外。目标已存在时命令默认拒绝覆盖，确认后才添加 `--force`；`--dry-run` 只报告 mode、目标路径和变量数量，不显示值，也不写文件。

导出文件不含 Shine 元数据，也不依赖 Shine 运行。若要停用 Shine env，请逐个导出并验证所需 mode，将含 secret 的输出加入 `.gitignore`，移除 `shine env run` 包装，最后再自行归档或删除 `shine.workspace.toml` 与对应的 `*.shine.toml` 环境源。导出命令不会删除这些源文件。

## 使用分层项目环境

在项目根目录创建 `shine.workspace.toml`，声明可用 mode、按顺序合并的文件和项目共享的
GPG recipient：

```toml
version = 2

[env]
modes = ["development", "production"]
default_mode = "development"
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
gpg_recipients = ["user@example.com", "team-backup@example.com"]
# 团队使用 age 时，取消以下两行注释，并填入每位成员的 recipient
# backend = "age"
# age_recipients = ["age1se1qexample...", "age1qteammate..."]
```

后面的环境文件覆盖前面的文件。`{mode}` 会替换为 `--mode` 指定的值；省略 `--mode`
时使用 `default_mode`。环境源文件可以同时包含明文值和待封存的 secret：

```toml
version = 1

[plain]
VITE_APP_NAME = "Example App"

[secret]
DATABASE_URL = true
API_TOKEN = false
SENTRY_TOKEN = "<待封存的值>"

[payload]
data = "<由 Shine 管理的 GPG 密文>"
```

- `true` 保留 payload 中已有的密文值。
- `false` 会在下次 `seal` 时安全提示输入。
- 字符串会在封存后替换为 `true`，避免明文继续留在文件中。

封存待处理的 secret，再用合并后的环境启动命令：

```bash
shine env secret seal
shine env run --mode production -- bun run build
```

`seal` 默认处理 workspace 引用的环境文件。可用 `shine env secret seal <FILE>` 只处理一个文件，
或通过 `--workspace <FILE>` 指定其他 workspace；`-r/--recipient` 可临时覆盖接收者。

默认情况下，当前进程已经存在的环境变量优先于 workspace。设置
`env.override_process_env = true` 后改由 workspace 值覆盖；显式 `--with` 始终具有最高
优先级。

配置了可用的 GPG recipient 时，`env run` 会在系统缓存目录按 mode 保存 GPG 加密缓存。
workspace、源文件内容或文件顺序变化后缓存会自动重建；无需手工编译或删除缓存。

个人覆盖文件应加入 `.gitignore`：

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

不要提交含有尚未封存字符串的环境文件。可在提交前搜索 `[secret]` 项并确认它们都已变为
`true`。

环境文件结构和覆盖顺序见[配置参考](../reference/configuration.md)。
