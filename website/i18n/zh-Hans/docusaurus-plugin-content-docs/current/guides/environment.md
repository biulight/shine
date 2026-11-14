---
title: 管理环境变量与密钥
sidebar_position: 5
---

# 管理环境变量与密钥

Shine 可以保存预设模板变量，也可以使用 GPG 或 age 封存项目环境中的敏感值。封存后的
密钥既可以提供给本机子进程，也可以通过 SSH Secret Broker 由远端命令按需请求本机解密。
不要把真实密钥写进公开仓库或文档示例。

按需求选择流程：

- [设置预设变量](#查看和设置变量)，例如代理地址。
- [只向一个命令提供变量](#只向一个命令提供变量)，不改变当前终端。
- 使用 [GPG](#使用-gpg-加密值) 或 [age](#使用-age-identity) 加密敏感值。
- [共享项目环境](#使用分层项目环境)，或[导入已有 dotenv 文件](#从-dotenv-初始化工作区)。
- [在构建时使用加密文件](#workspace-secret-files)，或[向远端提供密钥](#向远端命令提供变量与密钥)。

不要把私有 identity 或尚未封存的秘密提交到版本控制。只为可信命令批准解密；接收值的命令
及拥有足够权限的其他进程都可能读取明文。

## 查看和设置变量

按需选择查看、设置或删除操作，不要将以下示例作为设置后立即删除的流程执行：

| 操作 | 命令 |
| --- | --- |
| 查看变量及其来源 | `shine env list` |
| 读取生效值 | `shine env get HTTP_PROXY_PORT` |
| 检查优先级和来源 | `shine env explain HTTP_PROXY_PORT` |
| 保存变量值 | `shine env set HTTP_PROXY_PORT 6152` |
| 删除所选位置保存的值 | `shine env delete HTTP_PROXY_PORT` |

`PROXY_NO_PROXY` 控制 `setproxy` 设置的 `NO_PROXY` 和 `no_proxy`，默认为
`localhost,127.0.0.1,::1`。修改它或其他代理变量后，`shine update` 会把已安装的
`proxy` shell 预设标记为可更新；运行 `shine upgrade` 应用新值。

内置图片命令默认使用 `IMAGE_QUALITY=80`、`IMAGE_MAX_WIDTH=1920`、
`IMAGE_MAX_HEIGHT=1080`。可用 `--quality`、`--width`、`--height` 只覆盖当次运行，也可用
`shine env set` 为当前机器保留不同默认值。

`shine env list` 隐藏敏感值；`shine env explain KEY` 隐藏所有值，并指出生效值来自哪个文件。
只在安全终端中使用 `--reveal`；它不会解密秘密。注入时优先使用 `KEY_SECRET`，再使用 `KEY`，
因此修改明文值不一定会改变命令收到的值。

如果 Shine 报告 TOML 格式错误，请修正提示的文件或字段后重试。

全局 `~/.shine/config.toml` 和项目 `shine.config.toml` 的 `[env]` 支持简写字符串，也支持
同时记录值和说明：

```toml
[env]
HTTP_PROXY_PORT = "6152"
API_URL = { value = "https://api.example.com", description = "API 地址" }
```

`value` 的使用方式与简写字符串完全相同；`description` 会显示在 `shine env list` 中。
对已有详细条目执行 `shine env set API_URL https://api.example.org` 时，Shine 会更新 `value` 并保留
说明。

写入目标由参数和当前目录确定，与有效值来自哪里无关：

| 参数 | 目标 |
| --- | --- |
| 无范围参数 | 当前项目 `shine.config.toml` 的 `[env]`，无项目时为全局配置 |
| `--global` | 全局 `config.toml` 的 `[env]` |
| `--project` | 最近项目的 `[env]`，无项目时报错 |
| `--env-file` | 所选或默认范围的 `shine.env.toml` |
| `--overlay` | 当前手动 overlay 的 `shine.env.toml` |

在 `--global`、`--project` 和 `--overlay` 中选择一个范围。需要写入该范围的环境文件时，
添加 `--env-file`；对 `--overlay`，加与不加都指向同一个文件。脚本中应明确指定写入位置，
避免依赖执行时的目录。

同名变量被更高优先级的文件覆盖时，不指定目标的写入会被拒绝。先用 `shine env explain KEY`
查找生效来源，再选择要修改的位置。写入低优先级文件不会覆盖高优先级的值，完成后请检查输出中的生效来源。

按需选择操作和写入位置：

| 操作 | 命令 |
| --- | --- |
| 写入全局配置 | `shine env set HTTP_PROXY_PORT 7890 --global` |
| 写入项目配置 | `shine env set HTTP_PROXY_PORT 8080 --project` |
| 写入项目 env 文件 | `shine env set HTTP_PROXY_PORT 8081 --project --env-file` |
| 写入手动 overlay | `shine env set HTTP_PROXY_PORT 6152 --overlay` |
| 从全局 env 文件删除 | `shine env delete HTTP_PROXY_PORT --global --env-file` |
| 加密当前生效的 `MY_TOKEN`，将密文以 `MY_TOKEN_SECRET` 保存到项目 env 文件中 | `shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET --project --env-file` |

上述加密命令会保留原配置层中的明文 `MY_TOKEN`。

`delete` 只删除目标文件实际存在的项，不删除其他层或同时删除 `KEY_SECRET`；不存在则报错。
删除恢复继承，不表示禁用变量，空字符串仍是合法值。Git 托管 overlay 镜像禁止直接写入；
应修改上游、使用手动关联的 overlay，或选择本地覆盖层。旧脚本若用 `--force` 执行这些写入，
请改用上面的明确范围参数。

环境变量写入期间请勿同时编辑配置。若提示文件发生变化，请检查后重试。
若提示“文件已保存，但有效状态核对失败”，说明写入已发生；再次修改前，请重新查询该值。

写入会保留注释和无关条目，拒绝符号链接及特殊文件。Unix 文件仅允许所有者访问，
但未加密的值仍然是明文。

不带 `[env]` 表头的全局、overlay 和项目 `shine.env.toml` 覆盖文件也支持这两种格式：

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "本地代理主机" }
```

可用的值格式、说明和覆盖顺序见[配置参考](../reference/configuration.md#env-条目格式与说明)。

修改用于模板渲染的值后，运行：

```bash
shine upgrade
```

## 只向一个命令提供变量

使用 `--with` 提供变量，不修改当前终端，也不创建 workspace 文件：

| 操作 | 命令 |
| --- | --- |
| 按原名称注入变量 | `shine env run --with MY_TOKEN -- bun run build` |
| 为子进程重命名变量 | `shine env run --with MY_TOKEN=API_TOKEN -- bun run build` |
| 注入多个变量 | `shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build` |
| 跳过工作区加载 | `shine env run --no-workspace --with MY_TOKEN -- bun run build` |

每个 `KEY` 都优先解密 `<KEY>_SECRET`，不存在时才读取明文 `<KEY>`。等号右侧是子进程中
的变量名。显式 `--with` 值优先于当前进程和 workspace 的同名变量。

`--no-workspace` 会完全跳过 `shine.workspace.toml` 查找，只合并当前进程环境和显式
`--with`；它不能与 `--workspace` 或 `--mode` 同时使用。

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
CLI）不会接受从命令行传入的 token，而是读取 `GH_TOKEN` 这类环境变量。

只代理你信任的命令，名称只能包含 ASCII 字母、数字、`-`、`_` 或 `.`。
请将 `~/.shine/bin/` 放在 `PATH` 的靠前位置。目标必须是 Shine 命令目录之外的可执行程序，
不能是另一个 Shine 代理或 Shine 自身。

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

安装后照常调用 `gh`。代理会为命令提供 `GH_TOKEN`，优先解密 `GH_TOKEN_SECRET`，没有密文时
才使用明文 `GH_TOKEN`，不会改变当前终端的环境。需要多个变量时重复使用 `--with`，
或用 `KEY=ALIAS` 为传给命令的变量改名。

代理始终使用安装时选定的 Shine 配置目录，之后修改 `SHINE_CONFIG_DIR` 也不会切换。
需要改用另一个配置目录时，重新运行 `env proxy install`。项目规则仍按运行命令时所在的目录选择。

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

启用期间，每个 Cargo 子命令及其后代进程都可能继承 token。`disable` 后仍可使用 Cargo，
但不再解密或注入变量。偶尔执行 yank 时，应优先采用上面的单次 `env run`。

Windows 上请使用 `gh`，不要使用 `gh.exe`：安装会拒绝 `.exe`、`.com`、`.bat`、`.cmd` 和
`.ps1` 后缀。此前安装的带后缀名称仍可用 `shine env proxy uninstall <COMMAND>` 移除。

安装或卸载时，Shine 会保留发生冲突的文件。重试前请检查提示的冲突。
操作失败或中断后，请检查代理和配置再继续使用，不要假定恢复已经完成。

默认规则保存在全局 `~/.shine/config.toml`。在含有 `shine.config.toml` 的项目内加入
`--project`，可将该命令的规则限定到项目；同一命令的项目规则会覆盖全局规则。
离开项目后使用已有的全局规则；若没有适用规则，命令仍可运行，但不注入变量：

```bash
shine env proxy install gh --with GH_TOKEN --project
shine env proxy list
```

需要保留代理、但暂时停止解密和注入时，可禁用规则：

| 操作 | 命令 |
| --- | --- |
| 禁用代理 | `shine env proxy disable gh` |
| 启用代理 | `shine env proxy enable gh` |
| 禁用项目代理 | `shine env proxy disable gh --project` |

不加 `--project` 时，启用/禁用只修改全局规则，项目规则仍可覆盖它。
如果提示规则已保存但无法确认生效状态，请检查配置并再次运行 `shine env proxy list`。

不再需要时，卸载命令代理及其全局规则：

```bash
shine env proxy uninstall gh
```

项目规则会保留，重新安装代理前请检查这些规则。

如果真实命令被升级、移动或删除，重新执行安装命令以记录新的目标路径。

## 向远端命令提供变量与密钥

通过 `shine ssh` 运行远端命令时，根据远端需要看到明文的范围选择方式：

| 目标 | 本机命令 | 明文可见范围 |
| --- | --- | --- |
| 转发普通变量 | `shine ssh --with API_URL dev` | 远端登录 shell 或指定命令 |
| 解密并直接转发一个密钥 | `shine ssh --with-secret API_TOKEN dev` | 远端登录 shell 或指定命令 |
| 由远端子命令按需请求本机解密 | `shine ssh --secret-broker ... dev` | 仅获准启动的远端子进程 |

直接转发密钥适合可信远端上的临时操作，但远端登录 Shell 和同账号进程可能读取明文。若解密私钥
必须保留在本机，而且只有指定远端子进程应接收明文，请使用 SSH Secret Broker；远端管理员或
同账号进程仍可能检查该子进程。

设置命令、本机策略、登记流程和完整安全边界见
[SSH 会话：密钥代理与文件传输](./ssh-transfer.md#按需向远端命令提供密钥)。

## 使用 GPG 加密值

请安装所选加密后端（`gpg` 或 `age`）及其所需的身份插件。

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

加密只需要接收者公钥；解密需要对应私钥。如果私钥保存在 YubiKey 中，请连接它并按提示输入
PIN 或触摸设备。`decrypt` 会输出明文，只应在安全终端中执行。加密不会删除原来的明文变量，
不再需要时请显式删除。

旧版的单值 `gpg_key_id` 已废弃。先用 `shine state migrate --dry-run` 查看，再运行
`shine state migrate` 将它转换为 `gpg_recipients`；工作区中旧的
`[env.encryption].recipient` 也会在 `env run` 或 `env secret seal` 需要使用时提示迁移。

需要导出到当前 shell 时：

按需选择保留原变量名或重命名：

```bash
export_code="$(shine env secret export MY_TOKEN)" && eval "$export_code"
export_code="$(shine env secret export MY_TOKEN --as API_TOKEN)" && eval "$export_code"
```

在 PowerShell 中，仅在导出成功后执行生成的代码：

```powershell
$exportCode = shine env secret export MY_TOKEN --as API_TOKEN
if ($LASTEXITCODE -eq 0) { Invoke-Expression $exportCode }
```

安装 `utils` shell 预设后，也可以使用 `shine-env-export MY_TOKEN --as API_TOKEN`。
导出的值会留在当前 Shell 中；只有一个命令需要时，优先使用[单次注入](#只向一个命令提供变量)。
含 NUL 的值不能导出为环境变量。

## 使用 age identity

使用 age 可将项目秘密加密给一个或多个接收者（recipient）。已有 GPG 密文仍可读取，无需转换。

先确认 age 1.3 或更高版本已安装并位于 `PATH` 中。生成一个普通软件 identity，并记录输出中的
recipient：

```bash
shine env secret identity init
shine env secret identity list
```

普通 identity 默认写入 `~/.shine/age/identity.txt`。可用 `--output` 选择其他路径，
再按命令提示将其加入 `age_identity` 或 `age_identities` 配置。TOML 中使用的路径必须为
有效 UTF-8。`identity list` 显示已配置的身份路径和 recipient。

替换已有身份需要 `--force`。若仍需用旧身份解密历史数据，请先妥善备份。
Unix 身份文件仅允许所有者访问。

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

`--touch-id` 只适用于 macOS。解密需要原来 Mac 的 Secure Enclave，并按所选访问策略授权，
默认使用 Touch ID。即使 identity 文件被复制到另一台机器，通常也不能直接解密。看到意外的授权提示时，
应取消授权并检查触发它的命令。如果 AI Agent 可以在本机运行命令，请继续阅读
[在 AI Agent 参与开发时保护环境密钥](./agent-secret-safety.md#touch-id-改善什么)，了解这种方式的安全边界。

新建 Touch ID identity 会输出 `age1tag...` recipient。age 1.3 可以在 macOS、Linux 或
Windows 上直接向这个公开 recipient 加密，无需在加密端安装 `age-plugin-se`；只有解密端需要
插件和原来的 Mac。Tagged recipient 以可识别性换取这种跨平台能力：已经知道 recipient 的人
可以判断某份密文是否发给它。

如需在本机所有项目中使用同一默认后端和 recipient，将它们写入 `~/.shine/config.toml`：

```toml
secret_backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]
```

`age_identity` 指定一个身份，`age_identities` 可添加更多身份，包括硬件保护的身份。
这些文件应保存在需要解密的设备上。

如果 recipient 是某个项目团队共享的名单，应将它写入项目根目录的
`shine.workspace.toml` 的 `[env.encryption]`；这样可以随项目提交，而不会影响本机的其他
项目。该配置会优先于全局默认值，完整格式见下文“使用分层项目环境”。不要将私有
`age_identity` 提交到仓库。

也可以只在单次命令中选择后端和 recipient：

```bash
shine env secret encrypt --backend age -r age1tag1qexample... -r age1qteammate... --from MY_TOKEN
shine env secret seal --backend age -r age1tag1qexample... -r age1qteammate...
```

`-r/--recipient` 对 GPG 和 age 都可以重复使用。

旧版 Secure Enclave identity 可能使用 `age1se...` recipient，导致仅执行加密的电脑也必须
安装 `age-plugin-se`。在 workspace 所在项目中预览并应用公开 recipient 转换：

| 操作 | 命令 |
| --- | --- |
| 预览旧状态迁移 | `shine state migrate --dry-run` |
| 审阅预览后应用迁移 | `shine state migrate` |

迁移将配置中的 `age1se...` 转为等价的 `age1tag...`，不会解密密钥。
迁移后请重新封存，以使用新 recipient。已有密文保持不变，仍可解密。

### 为已有 workspace 密钥添加接收者

仅修改 `age_recipients` 不会更新已有密文。在 `shine.workspace.toml` 中保留原有接收者，
追加新成员的公开 recipient：

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1existing...", "age1newmember...", "age1backup..."]
```

将所有占位符替换为实际 recipient，然后在本机 identity 仍能解密旧 payload 的设备上运行：

```bash
shine env secret seal
```

`seal` 会解密旧 payload，再按当前完整接收者名单重新加密。即使 `[secret]` 中所有项都是
`true`，也会保留原值并重新加密；保持这些项为 `true` 即可，无需导出明文或重新输入值。
新增成员只需提供公开 recipient，不需要提供私有 identity。解密旧 payload 时，仍可能
要求 Touch ID、手机授权或对应 identity 的其他确认。

不指定文件时，`seal` 会处理 workspace 在已配置的各个 mode（包括 `default_mode`）下
引用的已存在源文件。使用 `shine env secret seal <FILE>` 可只处理一个源文件。如果封存
因错误中止，部分文件可能已更新；应解决报告的错误并重新运行，再共享结果。

将更新后的 workspace 配置和重新封存的共享源文件一起提交。名单中的每个接收者都能
独立解密新密文；请新成员在自己的设备上验证访问。新接收者无法解密 Git 历史中的旧密文。
同样，移除接收者并重新封存也不会撤销其对历史密文的访问，或轮换上游服务中的真实凭据；
必要时应在上游服务中轮换已泄露的凭据。

### 在 Windows 和 macOS 上实验手机授权 {#在-windows-上实验手机授权}

[`age-plugin-phone`](https://github.com/biulight/age-plugin-phone) 集成仍属实验功能，
只用于合成或可丢弃数据，不用于真实或生产秘密。保留独立验证过的恢复接收者，且恢复路径
不能依赖同一部手机、同一台电脑或同一份插件状态。

安装 age 1.3+，以及版本匹配且支持 tagged recipient 的桌面插件和手机应用。硬件要求、
支持的传输方式、安装与恢复步骤以插件的
[Windows 快速入门](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-beta-quickstart.md)或
[macOS 快速入门](https://github.com/biulight/age-plugin-phone/blob/main/docs/macos-quickstart.md)为准。
不要把私有测试签名根证书导入系统信任库。

通过 Wi-Fi 配对时，先在手机应用中打开 **Pair · Wi-Fi**，再运行：

```powershell
shine env secret identity init --phone --label "Work laptop" --transport auto
```

`auto` 先查找 Wi-Fi 手机；没有响应时，Windows 会选择 Developer USB/ADB，macOS 会选择 QR。
本指南只覆盖 Wi-Fi 和 Android ADB。请保持手机 Wi-Fi 监听就绪，或显式选择 ADB；
此流程中不要使用 tagged recipient 的 QR 回退方式。USB 配对的顺序相反：先启动桌面命令，等它提示连接手机，
再在手机选择 **Pair · USB**：

```sh
shine env secret identity init --phone --label "Work Mac" --transport adb --adb-serial SERIAL
```

连接多台 ADB 设备时用 `--adb-serial` 指定一台。若提示设备不唯一或发现失败，请解决后再试。
配对中断时，按插件文档使用 `setup --resume` 或 `setup --cleanup`，不要另起一次配对代替恢复。

指纹确认成功后，Shine 将身份路径加入用户的全局 `age_identities`。如果当前项目覆盖了
`age_identity` 或 `age_identities`，请先处理覆盖配置再配对。若配对成功但保存配置失败，
按 Shine 输出的配置手工补充即可，已有配对仍然有效。

该命令不会修改 `secret_backend` 或添加接收者。将输出的 recipient 和验证过的恢复 recipient
加入项目的 `shine.workspace.toml`：

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1tag...", "age1..."]
```

默认 tagged recipient 允许其他电脑仅用 age 1.3+ 加密，无需手机插件，也不会触发手机提示。
解密仍需要插件、匹配的手机应用和新的手机生物验证。知道 tagged recipient 的人可以判断密文是否
发给它。如需插件的 `age1phone...` 格式，使用
`shine env secret identity init --phone --recipient-type phone`；该格式也要求每台加密电脑安装插件。

已有配对无需重建。先确认桌面插件与手机应用支持 tag，再导出公开 tagged recipient：

```powershell
age-plugin-phone recipients -i <IDENTITY_STUB> --recipient-type tag
```

替换配置中对应的 recipient，保留恢复 recipient，再[重新封存工作区秘密](#为已有-workspace-密钥添加接收者)。
仅导出 recipient 不会更新已有密文；`shine state migrate` 不转换手机 recipient。
重新封存可能需要手机授权，也不会撤销对历史密文的访问。

后续通过 Wi-Fi 解密时，启用 **Wi-Fi auto-listen** 并保持手机应用在前台。需要插件提示时设置
`AGE_PLUGIN_PHONE_MESSAGES=1`。若配置多个手机身份后解密失败，检查插件更新，或临时只选择
匹配的身份；保留原配对与恢复状态。

如果 AI Agent 能在本机执行命令，使用前请阅读
[在 AI Agent 参与开发时保护环境密钥](./agent-secret-safety.md)。

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
# age_recipients = ["age1tag1qexample...", "age1qteammate..."]
```

后面的环境文件覆盖前面的文件。`{mode}` 会替换为 `--mode` 指定的值；省略 `--mode`
时使用 `default_mode`。环境源文件可以同时包含明文值和待封存的 secret：

```toml
version = 1

[plain]
VITE_APP_NAME = "Example App"

[secret]
DATABASE_URL = false
API_TOKEN = false
```

`false` 会在封存时提示输入值。Shine 加密后会将该项改为 `true`，并自动保存密文；无需手写
`[payload]`。已有的 `true` 项会在再次封存时保留原值。也可把待封存的值写成字符串，
但它在封存前仍是明文，不能提交到仓库。

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

个人覆盖文件应加入 `.gitignore`：

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

不要提交含有尚未封存字符串的环境文件。可在提交前搜索 `[secret]` 项并确认它们都已变为
`true`。

环境文件结构和覆盖顺序见[配置参考](../reference/configuration.md)。

## 从 dotenv 初始化工作区

已有 Vite 风格的 `.env` 文件时，可在项目根目录生成 Shine workspace 和对应的 TOML 环境源。
导入前先列出全部敏感键，并为每个键添加 `--secret`。未标记的值会进入 `[plain]`；
标记的值会进入 `[secret]`，但在封存前仍是明文。

下面以 `DATABASE_URL` 为敏感键。请替换为实际键名；只有全部导入值都不敏感时，才省略 `--secret`：

| 操作 | 命令 |
| --- | --- |
| 预览导入 dotenv 文件 | `shine env workspace init --from-dotenv --secret DATABASE_URL --dry-run` |
| 执行已审阅的导入 | `shine env workspace init --from-dotenv --secret DATABASE_URL` |

它读取 `.env`、`.env.local`、`.env.<mode>` 与 `.env.<mode>.local`，自动发现 mode，并保持该覆盖顺序。
只导入指定 mode 时，在预览和导入命令中都添加 `--mode`，可重复使用，
例如 `--mode development --mode production`。`--dry-run` 不创建或替换文件。
已有目标默认保留，只有确认要替换时才添加 `--force`。

导入后，检查生成的环境源是否仍有敏感值落在 `[plain]` 中。
按[使用分层项目环境](#使用分层项目环境)在 `shine.workspace.toml` 中配置加密后端和 recipient，再封存：

```bash
shine env secret seal
```

Unix 上，生成的 workspace 与环境源文件仅允许所有者访问。不要提交尚未封存的秘密。
原 dotenv 文件保持不变，其中的凭据仍是明文；即使 Shine 环境源已经封存，也要继续妥善保管这些原文件。

为避免改变 dotenv 语义，含 `$`（包括 `$BASE_URL` 和 `${BASE_URL}`）或带转义的双引号内容的值会被拒绝，即使它们位于引号内。先解析插值再导入；需要字面量美元符号时，请手动编写 Shine 环境源。单行反引号值会保留其中的 `#`、引号、反斜杠和空白；跨行引号值会被拒绝。未加引号的 `#` 表示注释开始；值本身含 `#` 时请加引号。

## 将 workspace 导出为 dotenv

当其他工具需要普通 dotenv 文件，或者准备停止使用 Shine env 时，可导出某个 mode 合并后的最终结果：

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local
```

`--format` 为必填项。导出包含所选 workspace 环境，不包含当前进程变量或 `--with` 值。默认只导出最终生效的 `[plain]` 项，不会解密 payload；若后层 secret 覆盖了前层 plain，同名旧明文也不会被导出。

只有目标确实需要完整可运行环境时，才显式包含已经封存的 secret：

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local \
  --include-secrets
```

这会将密钥以明文写入文件，请将输出排除在版本控制之外；Unix 上仅允许所有者读取。
已有目标默认保留，只有添加 `--force` 才允许替换。`--dry-run` 预览目标和非机密变量，不打印值、
不解密、不写文件，即使指定 `--include-secrets` 也一样。

导出文件不含 Shine 元数据，也不依赖 Shine 运行。若要停用 Shine env，请逐个导出并验证所需 mode，将含 secret 的输出加入 `.gitignore`，移除 `shine env run` 包装，最后再自行归档或删除 `shine.workspace.toml` 与对应的 `*.shine.toml` 环境源。导出命令不会删除这些源文件。

导出使用 dotenv 引号保留反斜杠、引号和换行原值。含 `$`、回车、NUL 字节或无法安全引用的值会在写入或替换输出前被拒绝。遇到这些值时，请继续使用 `shine env run`，或为目标工具单独准备输出文件。切换前务必用该工具验证导出结果。

## 与 GPG 和 age 用户共享秘密 {#让-gpg-与-age-成员共享一份-payload}

从 Shine 2.0.x 转换工作区前，先将所有读取端（包括 SSH broker 所在本机）升级到支持
hybrid 的版本。旧读取端无法读取 `hybrid:` 密文。封存需要 GnuPG 2.2–2.5 和 age 1.3+，
读取端只需所选后端。既有未标记 GPG 和 `age:` 密文继续
可读，普通读取不迁移文件。显式配置工作区，替换以下公钥占位符：

```toml
[env.encryption]
backend = "hybrid"
gpg_recipients = ["<full 40-hex primary fingerprint>"]
age_recipients = ["age1..."]
```

在能够解密已有秘密、并已安装两种加密工具的电脑上运行
`shine env secret seal --workspace shine.workspace.toml`。之后任一接收者组都可独立解密共享秘密。
GPG 接收者须使用完整主密钥指纹，不接受组名或不完整名称，也不会加入本机 GPG 设置中隐含的接收者。

每次使用仍须遵循所选后端的授权要求。解密失败或取消时，命令会停止，不会改用另一后端。
升级或修改工作区配置后，首次运行可能需要授权更多源文件，无需仅为此重新封存已有源文件。

可在选定的 `shine.workspace.toml` 旁创建个人配置 `shine.config.local.toml`，
使用 `--workspace` 时也以该工作区所在目录为准：

```toml
hybrid_decrypt_backend = "age"
```

值可以是 `gpg` 或 `age`。将 `/shine.config.local.toml` 加入项目的 `.gitignore`。
本机运行、封存和导出命令使用此设置；未设置时使用全局偏好，或提示选择可用后端。
能够修改该文件的程序也能改变所用授权方式。SSH broker 请求使用实际解密机器自己的选择。

封存只接受普通源文件，拒绝符号链接；Unix 上也拒绝具有多个硬链接的文件。
封存期间，包括等待硬件授权时，请勿编辑工作区或源文件。失败时，之前的文件可能已更新，
待封存明文也可能仍在。重试或提交前请检查提示的文件。封存仍在进行时不要删除
`.shine-seal.lock`。

## 临时机密文件 {#workspace-secret-files}

启用临时机密文件前，请将所有读取同一工作区的客户端升级到 Shine 2.5 或更高版本。

使用 workspace version 3，可将签名 keystore、密码文件和服务凭据加密后保存在 Git 中。
按用途为文件分组：`source` 是密文文件，`target` 是普通工具读取解密内容的位置。
使用 `--file-group` 选择一个或多个组。

将接收者占位符替换为实际公钥，解密身份保存在仓库外：

```toml
version = 3

[env]
default_mode = "staging"
modes = ["staging", "production"]

[env.encryption]
backend = "age"
age_recipients = ["<actual age recipient>"]

[[file_groups.android-signing.files]]
source = ".secrets/{mode}/release.jks.enc"
target = ".android-signing/release.jks"

[[file_groups.android-signing.files]]
source = ".secrets/{mode}/signing.properties.enc"
target = ".android-signing/signing.properties"

[[file_groups.service-account.files]]
source = ".secrets/{mode}/service-account.json.enc"
target = "app/.credentials/service-account.json"
```

路径相对 workspace 所在目录解析。`{mode}` 使用 `--mode` 指定的值，省略时使用
`env.default_mode`；所选环境必须已在工作区中声明。缺失文件会停止操作，不会回退到其他环境。
如果工作区只有文件组、没有 `env.files`，运行 `env run` 时必须显式选择 `--file-group`。

### 准备并封存文件

在 Git 仓库中，先将所有明文目标加入 `.gitignore`，例如 `.android-signing/` 和
`app/.credentials/`。目标必须在各自所属仓库（包括嵌套仓库）中未被跟踪且已被忽略。
Shine 不修改 `.gitignore`；遇到 Git 检查错误时，请解决后再继续。文件组也可用于 Git 仓库之外。

准备好明文目标文件后，先预览，再封存：

```bash
shine env secret seal --mode production --file-group android-signing --file-group service-account --dry-run
shine env secret seal --mode production --file-group android-signing --file-group service-account --remove-plaintext
```

封存会读取每个 `target`，按工作区加密设置写入对应密文 `source`。默认保留明文，只有添加
`--remove-plaintext` 才会删除。该选项需要可用的解密身份，并在删除前验证全部已写入密文；
删除前验证失败或取消时，原始明文全部保留。后续写入或删除中途失败可能留下部分结果，
重试前请检查报告的文件。文件组封存必须有明文目标，修改接收者时也一样。

### 使用临时文件运行命令

```bash
shine env run --mode production --file-group android-signing --file-group service-account -- pnpm android:build
```

Shine 在全部选中文件解密后才启动命令，并拒绝覆盖已有目标。命令结束或正常中断时，
Shine 尝试删除本次创建的文件；运行期间被替换的文件会保留并报告。请检查清理警告，
确认是否仍有明文残留。

不要启动需要在命令结束后继续使用这些文件的脱离后台任务，剩余受管子进程会被终止。
解密文件中的改动不会自动加密回仓库。

### 保留解密文件供其他工具使用

如果希望直接使用普通工具，而不必每次通过 Shine 启动，可导出选中文件组：

```bash
shine env workspace export --format files --mode production --file-group android-signing --file-group service-account --dry-run
shine env workspace export --format files --mode production --file-group android-signing --file-group service-account
```

`--mode` 必填。此操作向配置的目标路径导出文件，不导出环境变量；全部选中文件解密成功后
才开始写入。已有目标会保留，请先检查并移走后重试。`files` 格式没有 `--force`、`--output`
或 `--include-secrets` 选项。工作区配置和密文源保持不变。

导出成功后，文件会持续保留，直到你自行删除。`env run --file-group` 会拒绝这些已有目标，
不会接管清理。导出失败或中断时，Shine 尝试移除本次创建的文件和空目录，保留被替换的文件，
并报告可能残留的内容。重试前请检查这些警告，不要将明文提交到仓库。

### 限制与注意事项

- 明文文件仅允许当前用户访问；Windows 文件系统须支持这种私有权限。命令本身和拥有足够
  本机权限的其他进程仍可读取这些文件。
- 路径必须位于工作区内。链接、重叠映射和与工作区定义文件冲突的映射会被拒绝；每个密文源与
  明文目标都应使用独立路径。
- 单个明文文件最大为 16 MiB，单次总量为 64 MiB；密文文件分别限制为 32 MiB 和 128 MiB。
  这些密文应使用 Shine 读取，不是原生 `.age` 或 `.gpg` 文件。
- `--dry-run` 只检查配置和元数据，不读明文、不解密、不写入、不启动命令，也无法验证密文
  完整性或私钥可用性。
- 必须显式选择文件组。`--file-group` 不能与 `seal <FILE>` 同用，也不能与 `env run` 的
  `--no-workspace`、`--secret-broker` 同用。持久文件导出支持下文的 `--secret-broker`。
- 强制结束 Shine 或断电可能留下明文，重试前请检查目标。删除文件不代表安全擦除，移除
  接收者也不能撤销其对 Git 历史密文的访问。

### 通过 SSH broker 导出

为 `env workspace export --format files` 添加 `--secret-broker`，即可在启用了
`shine ssh --secret-broker` 的会话中请求本机授权和解密。该选项要求明确选择文件组，
并匹配文件策略，不适用于 dotenv 导出。未指定时，手动导出仍使用执行机器的后端。
参见 [SSH 文件授权](ssh-transfer.md#file-secret-broker)和
[App 自动秘密文件安装](app-presets.md#managed-secret-files)。

## Env 分层变更的人工升级与回退

从 2.3.x 或更早版本升级时，请按[升级与迁移](./upgrading.md#从-23x-或更早版本升级环境分层)检查变量优先级、
更新脚本并准备回退。
