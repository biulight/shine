---
title: 配置参考
sidebar_position: 2
---

# 配置参考

Shine 将全局配置和状态保存在 `~/.shine/`，需要时创建 `config.toml`。
Unix 上，若 `SUDO_USER` 指明发起 sudo 的用户，主目录相对路径使用该用户的主目录。

本页用于查询配置字段和覆盖顺序。具体操作见[环境变量与密钥](../guides/environment.md)或
[自定义预设](../guides/custom-presets.md)。

## 常用全局字段

在 `~/.shine/config.toml` 中只添加所需配置，并替换示例路径与接收者。
敏感值应先加密，不要直接写入 `[env]`。

```toml
presets_dir = "~/dotfiles/shine-presets"
# 预设开发时才设为 live
external_shell_mode = "snapshot"
presets_overlay_git = "https://example.com/team/shine-overlay.git"
presets_overlay_git_branch = "main"
app_default_dest_root = "~/.config"
sync_terminal_theme = true
gpg_recipients = ["user@example.com", "team-backup@example.com"]

secret_backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]

[env]
HTTP_PROXY_PORT = "6152"
SOCKS5_PROXY_PORT = "6153"
PROXY_HOST = "127.0.0.1"
PROXY_NO_PROXY = "localhost,127.0.0.1,::1"
IMAGE_QUALITY = "80"
IMAGE_MAX_WIDTH = "1920"
IMAGE_MAX_HEIGHT = "1080"
API_URL = { value = "https://api.example.com", description = "API 地址" }

[[env_proxy]]
command = "gh"
with = ["GH_TOKEN"]
# 省略时为 true；设为 false 时照常运行命令，不解密也不注入值
enabled = false
```

| 字段 | 作用 |
| --- | --- |
| `presets_dir` | 使用完整的外部预设目录替代内置预设 |
| `external_shell_mode` | 外部 Shell 预设的部署方式；默认 `snapshot`，可显式设为 `live` |
| `presets_overlay_dir` | 覆盖同路径预设文件的本地目录；不能与 `presets_overlay_git` 同用 |
| `presets_overlay_git` | 由 Shine 管理在 `~/.shine/overlay/` 下的 Git overlay URL；仅用于全局配置 |
| `presets_overlay_git_branch` | Git overlay 跟踪的分支；省略时使用远端默认分支 |
| `app_default_dest_root` | 未声明目标路径的旧式 app 预设默认根目录 |
| `sync_terminal_theme` | 受管 Unix Shell 的自动终端主题同步，默认启用；仅用于全局配置 |
| `gpg_recipients` | `shine env secret encrypt` 默认 GPG recipient 列表 |
| `secret_backend` | 默认密钥后端，省略时为 `gpg` |
| `age_recipients` | `age` 后端默认加密接收者列表 |
| `age_identity` | 解密 `age:` 密文时使用的身份文件路径，省略时可使用 `~/.shine/age/identity.txt` |
| `age_identities` | 附加 identity 文件路径列表，排在 `age_identity` 之后并自动去重 |
| `[env]` | 模板变量及 shell helper 使用的值 |
| `[[env_proxy]]` | 一个透明命令代理规则；`command` 为裸命令名，`with` 为允许注入的 `KEY` 或 `KEY=ALIAS` 列表，`enabled` 默认为 `true` |

旧的 `gpg_key_id`、`[env.encryption].recipient` 或 `age1se...` 接收者可用
`shine state migrate --dry-run` 预览转换，审阅后运行 `shine state migrate`。普通读取不会改写这些配置。
重新封存前，请阅读[加密设置](../guides/environment.md#使用-gpg-加密值)和
[age identity](../guides/environment.md#使用-age-identity)。
手机接收者需要单独[手动转换](../guides/environment.md#在-windows-上实验手机授权)。

## 外部代码信任

执行外部预设代码前，请审阅它要执行的操作和访问范围。交互确认可以只授权本次列出的代码，
不保存长期信任；自动化与 `--yes` 需要已有授权。Live Shell 预设要求开发信任，
这也会授权未来的代码修改。显式 `--run-generators` 检查同样需要已有授权。

使用以下命令检查和管理信任。不要手改 `~/.shine/trust.toml`，也不要把授权写入项目配置。

| 操作 | 命令 |
| --- | --- |
| 仅检查，不授予信任 | `shine trust inspect app/example` |
| 授予快照信任 | `shine trust grant app/example` |
| 查看授权 | `shine trust list` |
| 撤销信任 | `shine trust revoke app/example` |
| 仅检查，不授予信任 | `shine trust inspect shell/tools/my-command` |
| 授予快照信任 | `shine trust grant preset` |
| 信任未来的代码变更 | `shine trust grant app/example --development` |

快照授权涵盖目标所在的整个预设类别，以及已审阅的目标、来源和能力；变化后需要重新审阅。`--development`
允许所显示本地来源中的代码继续修改，但目标、能力、来源目录或来源层变化后仍须授权。
它不代表持续代码审阅，也不能替代每次操作前的计划确认。

`preset` 覆盖当前生效的目标，不包括将来新增的目标。旧授权即使仍可查看和撤销，也可能需要
重新审阅。已停用的 `allow_app_hooks` 和 `allow_sys_code` 不再授予信任，请使用上面的信任命令。

## Env 条目格式与说明

全局 `config.toml`、项目 `shine.config.toml` 的 `[env]`，以及各层 `shine.env.toml` 覆盖
文件，都支持两种等价的值格式：

```toml
[env]
PLAIN_VALUE = "example"
DETAILED_VALUE = { value = "example", description = "供构建任务使用的示例变量" }
```

- 字符串适合不需要补充说明的变量。
- 详细格式中的 `value` 参与 `env get`、模板替换、`env secret encrypt`、`env secret export` 和
  `env run --with`，行为与字符串格式相同。
- `description` 只用于 `shine env list` 的可读说明，不会传入子进程或模板。
- 当前配置中的内联 `description` 优先于预设 `<presets>/env.toml` 中的同名说明。
- `shine env set` 更新已有详细条目时会保留其 `description`。
- 字符串与仅有 `value` 的详细项只覆盖值并继承低层说明。只有显式 `description` 才替换说明；
  空字符串清空说明并阻止使用预设中的默认说明。
- 数字、数组、缺少字符串 `value` 等无效类型会报告文件路径和变量名，不会被静默忽略。

## 项目配置

Shine 从当前目录向上查找最近的 `shine.config.toml`。项目配置覆盖全局配置，未声明的字段继续
使用全局值；相对路径以声明它的配置文件所在目录为基准。

`[[env_proxy]]` 也遵循项目覆盖规则：项目内同名 `command` 的规则会取代全局规则，其它全局代理规则继续生效。
优先用 `shine env proxy install`、`enable` 和 `disable` 管理这些条目，保持配置与
`~/.shine/bin/` 中已安装的命令入口一致。

如果仍在使用旧的 `config.toml`、`.env.toml` 或全局 `env.toml`，请按
[旧配置恢复步骤](../troubleshooting.md#升级到-040-后提示旧配置文件)处理。

## 目录与来源优先级

相对的 `--config-dir`、`SHINE_CONFIG_DIR` 和 `SHINE_PRESETS` 路径以运行 Shine 时所在的目录为基准。

显式传入的 `--config-dir` 会在本次调用中覆盖 `SHINE_CONFIG_DIR`。下文的 `SHINE_CONFIG_DIR`
均指最终选定的目录，也包括通过 `--config-dir` 指定的目录。它会改变全局配置和运行时状态目录；不在 Shine 项目中时，
预设目录也固定为 `$SHINE_CONFIG_DIR/presets/`，此时 `SHINE_PRESETS` 和全局 `presets_dir`
都不能覆盖它。

如果当前目录存在生效的项目 `shine.config.toml`，`SHINE_CONFIG_DIR` 仍决定运行时状态目录，
但显式设置的 `SHINE_PRESETS` 或项目 `presets_dir` 可以选择预设来源；从全局配置继承的
`presets_dir` 不能覆盖 `$SHINE_CONFIG_DIR/presets/`。

未设置 `SHINE_CONFIG_DIR` 时，基础预设目录按以下顺序选择：

1. `SHINE_PRESETS`
2. 项目 `shine.config.toml` 中的 `presets_dir`
3. 全局 `config.toml` 中的 `presets_dir`
4. 默认 `~/.shine/presets/`

外部预设或 overlay 根目录可以是符号链接；更改链接目标后需要重新审阅，失效的根目录链接会报错。
预设树内部的链接不会被包含，请使用普通文件和目录；套接字、设备等特殊文件会被拒绝。

外部 Shell 预设默认使用 `snapshot` 模式：修改来源后，需运行 `shine update` 和
`shine upgrade` 才能应用。仅在开发时使用 `external_shell_mode = "live"`；该模式下来源修改
会在下次调用时生效，必须授予 development trust。`target`、`runtime`、`transforms` 或 `env`
变化后仍需运行 `shine upgrade` 更新已安装命令。切换 snapshot/live 模式也需要更新已安装命令，
步骤见[外部 Shell 部署方式](../guides/custom-presets.md#选择外部-shell-的部署方式)。

升级 Shine 后，请用 `shine upgrade` 更新已有 Live Bun 命令。使用 transforms 的命令沿用
安装时选择的配置目录，项目覆盖仍取决于调用时的工作目录。

Overlay 在选定的基础预设来源上按相同相对路径覆盖文件，不替代整棵目录。手动关联的
`presets_overlay_dir` 与 `presets_overlay_git` 互斥；使用 `shine preset overlay link` 可避免同时配置。
`presets_overlay_git` 和 `presets_overlay_git_branch` 仅用于全局 `config.toml`；
请在 Shine 项目之外运行 `overlay link --git`。项目可使用 `presets_overlay_dir` 关联本地 overlay。
关联命令克隆成功后，Git overlay 即可生效；手动填写配置时，则需先成功执行 `shine preset pull`。
本地检出会在后续拉取时强制镜像到远端状态，因此不要直接修改 `~/.shine/overlay/`。

## Env 值覆盖顺序

同名环境值按以下顺序合并，后者覆盖前者：

1. 内置默认值（仅运行时兜底）；
2. 当前 overlay 的 `shine.env.toml`（共享基线）；
3. 全局 `config.toml` 的 `[env]`；
4. 全局 `shine.env.toml`；
5. 项目 `shine.config.toml` 的 `[env]`；
6. 项目 `shine.env.toml`。

`SHINE_CONFIG_DIR` 可将全局根目录改为 `~/.shine/` 之外的位置。显式保存的值即使等于默认值，
也会继续生效；从对应层删除后才恢复继承。单独存在 `shine.env.toml` 不构成项目。
项目 env 文件只从最近 `shine.config.toml` 所在目录读取，不合并多个祖先项目。

用 `shine env explain KEY` 查看值来源、描述来源、覆盖链和默认写入目标；默认不显示值，
`--reveal` 才显示且不会解密。旧版本用户请参阅
[升级与回退步骤](../guides/upgrading.md#从-23x-或更早版本升级环境分层)。

`shine.env.toml` 是不带 `[env]` 表头的覆盖文件，例如：

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "本地代理主机" }
```

## Workspace 环境

`shine.workspace.toml` 定义项目可用 mode、环境源文件和项目共享的加密 recipient：

```toml
version = 2

[env]
modes = ["development", "production"]
default_mode = "development"
override_process_env = false
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
gpg_recipients = ["user@example.com", "team-backup@example.com"]
# 也可使用 age 后端
# backend = "age"
# age_recipients = ["age1tag1qexample...", "age1qteammate..."]
```

环境源按 `files` 顺序合并。默认保留当前进程已经存在的变量；设置 `env.override_process_env = true` 后，改由 workspace 值覆盖。

工作区加密设置覆盖全局默认值。执行 `shine env secret seal` 时，显式 `--backend` 和
`--recipient` 选项优先于工作区设置。团队接收者名单放在 `[env.encryption]` 中；recipient
是公开信息，但私有身份文件及本机 `age_identity`/`age_identities` 路径不能提交到共享仓库。

创建包含普通变量和待封存秘密的环境源文件：

```toml
version = 1

[plain]
PUBLIC_VALUE = "example"

[secret]
API_TOKEN = false
```

| 环境源条目 | 含义 |
| --- | --- |
| `[plain]` 字符串 | 非敏感值 |
| `[secret]` 值为 `false` | 下次运行 `shine env secret seal` 时提示输入 |
| `[secret]` 值为 `true` | 保留此文件中已封存的值 |
| `[secret]` 字符串 | 等待封存的明文，不能提交 |
| `[payload].data` | Shine 写入的密文，不要手工填入 |

封存会加密待处理值，并将对应条目改为 `true`。`shine env run` 按环境源顺序合并普通值和解密值。

`shine env run --with KEY[=ALIAS]` 还可注入当前 Shine 配置 `[env]` 中的值。它优先读取
`KEY_SECRET`，不存在时读取 `KEY`；显式注入值覆盖 workspace 和当前进程中的同名变量。

## 受管目录

以下路径位于全局根目录下，通常是 `~/.shine/`：

| 路径 | 用途 |
| --- | --- |
| `config.toml` | 全局配置 |
| `shine.env.toml` | 全局环境覆盖值 |
| `bin/` | 已安装命令入口，需要加入 `PATH` |
| `presets/` | 运行时预设，按 `app/`、`shell/`、`sys/` 分类 |
| `overlay/` | Shine 管理的 Git overlay，应修改上游仓库而非这份副本 |

Shine 还在这里保存安装记录和恢复文件。不要手工删除后再期望 Shine 重新识别旧安装，
请使用对应的 `uninstall --dry-run` 和 `uninstall` 命令。旧服务残留的 `http/` 文件请按
[HTTP 服务迁移步骤](../guides/upgrading.md#清理旧版本的本地-http-服务)处理。

## 工作区混合加密

`env.encryption.backend = "hybrid"` 要求工作区同时提供 `gpg_recipients` 和
`age_recipients`，每组至少一个非空项。GPG 项必须是本机已有公钥的完整 40 位十六进制
主密钥指纹。两组均不合并全局收件人；仅同时配置两组不会启用 hybrid。

在本机全局 `config.toml` 中设置 `hybrid_decrypt_backend = "gpg"` 或 `"age"`。
本机工作区操作可用工作区旁的 `shine.config.local.toml` 覆盖此字段；文件或字段缺失则继承
全局设置。共享的 `shine.config.toml` 不能覆盖或保存此偏好。将个人文件加入 `.gitignore`；
它只接受该字段，格式错误或未知字段会报错。此偏好只影响混合密文的解密选择，不授予秘密访问权限。
SSH broker 使用解密机器的选择，不接受远端个人配置。全局 `secret_backend` 只接受 `gpg` 或 `age`。

配置步骤和授权要求见[环境指南](../guides/environment.md)。

## Workspace 文件组

Workspace version 3 支持 `file_groups.<名称>.files`，version 2 不支持。

| 配置 | 要求 |
| --- | --- |
| 组名 | 字母、数字、短横线和下划线；每组必须包含文件 |
| `source` | 密文文件路径 |
| `target` | 明文文件路径 |
| 路径 | 相对工作区目录，可使用 `{mode}` |
| `env.files` | 声明文件组时可省略 |
| 加密设置 | 共享 `[env.encryption]`，不提供按组或按环境覆盖 |

封存、运行、导出、路径限制和明文清理步骤见[临时机密文件](../guides/environment.md#workspace-secret-files)。
