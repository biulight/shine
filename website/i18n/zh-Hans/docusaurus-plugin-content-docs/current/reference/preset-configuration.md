---
title: 预设配置（shine.toml）
sidebar_position: 3
---

# 预设配置（shine.toml）

编写类别的 `shine.toml` 时，可在本页查询字段。创建、关联、授权和验证类别的流程见
[自定义预设](../guides/custom-presets.md)。全局与项目设置属于[配置参考](./configuration.md)，
不应写入本文件。

| 位置 | 配置内容 |
| --- | --- |
| `app/<category>/shine.toml` | 应用文件、生成器、hook 与 artifact |
| `shell/<category>/shine.toml` | 安装到 Shell 的命令 |
| `sys/<os>/shine.toml` | 软件安装项、Shell 集成与受管系统设置 |

类别级字段应放在 `[artifact]`、`[[files]]` 或其他 TOML 表头之前。

## 共用规则 {#shared-rules}

### 平台与路径 {#platforms-and-paths}

App 与 Shell 的 `files.platforms` 接受非空数组，可选 `macos`、`linux`、`windows`、`unix`。
多个选择器满足任意一个即可，`unix` 包含 macOS 和 Linux。省略时不额外过滤平台，但目标目录
或原生脚本类型仍可能排除某个平台。

App 的 `dest` 接受绝对路径（包括 `~/.config/example` 这样的主目录相对路径），或平台映射。
精确平台键优先于 `unix`；缺少分支时，该类别或文件不会在对应平台出现。源路径相对于类别目录，
文件级覆盖不能启用类别目标已排除的平台。App 的 `target` 相对于选定的目标目录。这些路径不能包含 `..`。

App 文件级 `dest` 还支持 `{ base = "data-dir", path = "com.example.app" }`。
基准目录在 Windows 上是 `%APPDATA%`，macOS 上是 `~/Library/Application Support`，Linux 上是
`XDG_DATA_HOME`（未设置时为 `~/.local/share`）。`path` 与 `target` 必须是相对路径。
类别级 `dest` 不支持这种结构化形式。

### 脚本运行时 {#runtimes}

| 值 | 含义与要求 |
| --- | --- |
| `native`（默认） | 运行原生入口。Shell 命令在 Unix Shell 中使用 `.sh`，PowerShell 中使用 `.ps1`；App 脚本直接执行，需要适合该系统的可执行入口。 |
| `bun` | 使用 `PATH` 中已安装的 Bun。入口脚本支持 `.ts`、`.js`、`.mts`、`.mjs`。 |

适用于 Shell 的 `files.runtime` 以及 App 的脚本 hook、生成器和 artifact。
Sys 安装器没有 `runtime` 字段。Bun Shell 入口不能与 `needs_source = true` 组合。

Bun 的安装与语言行为见 [Bun 官方文档](https://bun.sh/docs)。外部预设使用 registry 包的方法见
[锁定依赖](../guides/custom-presets.md#在外部-bun-预设中使用锁定依赖)。

### 文件转换 {#transforms}

`transforms` 是按顺序执行的字符串数组，默认为空。支持以下值：

| 值 | 作用 |
| --- | --- |
| `template` | 用 Shine 环境值替换 `@@VARIABLE@@`；缺少变量时失败。不会按目标语言自动添加引号或转义。 |
| `jsonc-to-json` | 将 JSONC 输入转换为 JSON。 |

例如，`transforms = ["template", "jsonc-to-json"]` 先替换变量，再转换格式。
App 文件还支持单字符串简写 `transform`，不能与 `transforms` 同时使用。

### 环境输入 {#environment-inputs}

Shell Bun 入口及 App 的 hook、生成器、artifact 接受 `env` 数组。除生成器必须至少声明触发输入外，
其他入口默认为空：

```toml
env = [
  { source = "API_URL", sensitivity = "plain" },
  { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" },
]
```

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `source` | 必填字符串 | Shine 配置中的变量名。 |
| `target` | 与 `source` 相同 | 脚本或命令接收的变量名；同一列表内不能重复。 |
| `sensitivity` | 必填字符串 | `plain` 或 `secret`，声明供审阅使用的敏感度，不会加密值。 |

只填写变量名，不填写值或密文。名称以 ASCII 字母或 `_` 开头，后续可用 ASCII 字母、数字、`_`。
同一来源的敏感度声明必须一致。这些输入已提供环境权限声明，无需在 `permissions.environment`
中重复。旧的 `env = ["SOURCE=TARGET"]` 字符串数组需要[迁移](../guides/custom-presets.md#迁移-1x-来源)。

Shell 输入注入要求 `PATH` 中有 `shine`，优先读取 `SOURCE_SECRET`，其次是明文 `SOURCE`，
不会加入工作区的默认秘密绑定。App 输入按 `source` 的准确名称读取 Shine 配置，不会自动查找
`SOURCE_SECRET` 或解密其值。artifact 和脚本 hook 省略未配置的可选输入；生成器和命令 hook
要求其列出的输入均已配置。声明 `env` 不会将进程与继承的环境隔离。实际值的配置和加密见[环境指南](../guides/environment.md)。

## App 类别 {#app}

### 类别字段 {#app-category}

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `metadata_schema_version` | `1`（旧版） | 支持 `1`、`2`、`3`。新建普通预设使用 `2`；声明 `secret_files` 时使用 `3`。 |
| `description` | 无 | 类别的可读说明。 |
| `dest` | 必填 | 默认目标根目录，见[路径规则](#platforms-and-paths)。 |
| `list_mode` | 显式声明 `files` 时为 `files`，否则为 `category` | 控制 App 状态和更新输出按文件或类别分组。可用目录始终按类别展示。 |
| `files` | 自动发现类别文件 | 显式文件列表。省略时发现除 `shine.toml` 外的文件；`secret_files` 列表非空时不再隐式发现。使用显式条目避免把辅助脚本作为安装目标。 |
| `secret_files` | `[]` | 基于 workspace 的加密文件组，见[秘密文件](#secret-files)。 |
| `artifact` | 无 | 显式 apply/remove 操作使用的脚本。 |
| `post_install`、`post_upgrade` | 无 | 单个 hook 或非空 hook 数组，见[生命周期 hook](#hooks)。 |
| `permissions` | 无 | 类别级[能力声明](#permissions)。 |

### 文件条目：`[[files]]` {#app-files}

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `source` | 必填字符串 | 类别内已有的相对路径输入文件；声明生成器时也必须存在。 |
| `target` | 与 `source` 相同 | 相对于最终 `dest` 的安装路径。 |
| `dest` | 类别 `dest` | 为此文件覆盖目标目录，也支持结构化 data-directory 形式。 |
| `description`、`display_name` | 无 | 说明及可选显示名称，均不改变安装路径。 |
| `platforms` | 不额外过滤 | 支持的[平台选择器](#platforms-and-paths)。 |
| `transforms` / `transform` | 不转换 | 应用[转换流水线](#transforms)，两种形式互斥。 |
| `install_mode` | `copy` | `copy` 替换文件；`json-merge` 更新选定的 JSON 顶层键。两者均保留正常的归属与冲突检查。 |
| `managed_keys` | `json-merge` 时必填 | 非空数组，包含唯一、非空且不含点号的顶层键名；不能与 `copy` 同用。 |
| `requires_admin` | `false` | 声明安装此文件需要管理员权限，不会授予权限。 |
| `restart_hint` | 无 | 提示用户如何使改动生效的文本，不会重启应用。 |
| `generator` | 无 | 生成此文件的内容，见下文。 |

JSON 合并要求转换后的内容为对象，且包含所有受管键。每个受管值会整体替换，内部嵌套设置不会
独立保留；`managed_keys` 之外的键会保留。最终目标路径重复时会拒绝操作。更新、备份、卸载及
恢复行为见[管理应用配置](../guides/app-presets.md)。

没有 `shine.toml` 的旧 App 预设可在首行或 shebang 后一行使用 `# shine-dest:`，指定包含
文件名的完整目标路径。主目录展开后必须是绝对路径，且不能包含 `..`。新预设应在元数据中声明
`dest` 和 `target`。

### 文件生成器：`[files.generator]` {#generators}

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `script` | 必填字符串 | 类别内的相对路径脚本，其标准输出作为文件内容。 |
| `runtime` | `native` | [脚本运行时](#runtimes)。 |
| `env` | 必填非空数组 | 提供给此生成器的[输入](#environment-inputs)，必须包含 `when_env` 来源。 |
| `when_env` | 必填字符串 | 仅在 Shine 环境中存在此来源名称时运行；该名称也必须在本生成器的 `env` 中声明。 |
| `auto` | `true` | 允许升级时生成。设为 `false` 时使用显式安装或刷新，普通 update/upgrade 不会获取新的生成内容。 |
| `fallback` | 无 | 主生成器的 `when_env` 未配置且备用生成器的 `when_env` 已配置时，使用备用生成器。两者的触发变量须不同，`auto` 值须相同。备用生成器不能嵌套。 |

没有启用的生成器时使用源文件。检查默认不运行代码，除非显式要求。生成仍需批准或信任授权，
失败会保留上次成功安装的内容。刷新步骤见[管理应用配置](../guides/app-presets.md)。

### 辅助资源：`[artifact]` {#artifacts}

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `script` | 必填字符串 | 类别内的 apply 脚本相对路径。 |
| `teardown` | 无 | 类别内的清理脚本相对路径；执行 `remove` 时必需。 |
| `runtime` | `native` | 两个脚本共用的[运行时](#runtimes)。 |
| `env` | `[]` | 两个脚本可用的[输入](#environment-inputs)。 |

通过 `shine app artifact apply <CATEGORY>` 或 `remove` 运行。具体接入和清理由脚本决定，
元数据本身不提供回滚。操作见[辅助资源流程](../guides/tasks-and-serve.md#生成-app-辅助资源)。

### 生命周期 hook {#hooks}

`post_install` 与 `post_upgrade` 是类别级字段。每个 hook 支持：

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `command` / `script` | 必须且只能填一个 | 要运行的程序，或类别内的脚本相对路径。 |
| `args` | `[]` | 传给入口的参数字符串数组。 |
| `runtime` | 脚本默认为 `native` | 仅与 `script` 一起使用，见[运行时](#runtimes)。 |
| `show_output` | `false` | 操作显示成功输出时，显示成功 hook 的输出。 |
| `env` | `[]` | 此 hook 的[输入](#environment-inputs)。 |

触发时机与失败处理见[应用接入说明](../guides/app-presets.md#构建辅助资源)。
不要从 hook 调用 artifact 生命周期命令。

### 秘密文件：`[[secret_files]]` {#secret-files}

要求 App 元数据版本 `3`，以及 workspace 版本 `3` 的文件组。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `workspace` | 必填字符串 | 相对于类别目录的 workspace 文件路径。 |
| `file_group` | 必填字符串 | 该 workspace 中声明的文件组。 |
| `mode` | 必填字符串 | 已声明的 workspace mode，例如 `production`；不会隐式选择默认 mode。 |
| `auto_decrypt` | `false` | 安装或升级时释放并管理明文；`false` 只安装加密材料，不自动释放明文。 |
| `unix_uid`、`unix_gid` | 执行安装的用户/组 | Unix 上明文文件的数字用户/组 ID；更改归属可能需要管理员权限。 |
| `unix_mode` | `"0600"` | Unix 权限字符串，仅支持 `"0400"` 或 `"0600"`。 |

Windows 上省略全部 Unix 字段。明文目标由 workspace 映射提供。不要在 `files` 中重复声明
workspace/密文文件，不要把明文放入预设，也不能与其他受管目标重叠。秘密文件不支持生成器、
转换、JSON 合并或自动目标迁移。启用明文释放前先阅读[权限与恢复限制](../guides/app-presets.md#managed-secret-files)。

### App 脚本路径 {#app-script-paths}

生成器、脚本 hook、artifact 和 teardown 无需 `env` 声明即可获取下列变量；命令 hook
不会获得这组 App 专用变量。

| 变量 | 含义 |
| --- | --- |
| `SHINE_APP_ID` | 应用类别名称。 |
| `SHINE_APP_DIR`、`SHINE_APP_SOURCE_DIR` | 本次运行使用的文件目录，也是脚本工作目录。 |
| `SHINE_APP_OVERLAY_DIR` | 本次运行的 overlay 文件目录，没有则为空。 |
| `SHINE_CONFIG_DIR` | 当前 Shine 配置与状态根目录。 |
| `SHINE_STATE_DIR` | 此类别的持久输出目录。 |
| `SHINE_CACHE_DIR` | 此类别的缓存输出目录。 |

源目录是临时目录，不要保存其路径或向其中写入持久结果。需要保留的输出使用 state/cache
目录，必要时自行创建。

## Shell 类别 {#shell}

| 类别字段 | 默认值 | 含义 |
| --- | --- | --- |
| `description` | 无 | 类别说明。 |
| `permission_defaults` | 无 | 没有独立权限表的条目使用的[权限](#permissions)。 |
| `files` | 自动发现原生脚本 | 显式命令条目。省略时发现匹配的 `.sh`/`.ps1` 脚本；Bun 入口以及需要排除辅助脚本时使用显式列表。 |

每个 `[[files]]` 条目支持：

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `source` | 必填字符串 | 类别内已有脚本的相对路径。 |
| `target` | 源文件名去掉脚本扩展名 | 安装后的命令名，不是路径；当前类别内的命令名必须唯一。 |
| `description` | 脚本开头的注释 | 命令的可读说明。 |
| `needs_source` | `false` | 通过 Shell 函数调用原生入口，使其能改变当前会话；不能用于 Bun。 |
| `platforms` | 不额外过滤 | [平台选择器](#platforms-and-paths)，原生脚本还需匹配 Shell 类型。 |
| `runtime` | `native` | [脚本运行时](#runtimes)。 |
| `transforms` | `[]` | 按顺序执行的[转换](#transforms)。 |
| `env` | `[]` | [运行时输入](#environment-inputs)，仅支持 Bun 入口。 |
| `permissions` | 类别 `permission_defaults` | 替换该条目的默认权限表，不合并。 |

原生脚本开头附近的 `# shine-template: true` 也会在显式转换列表为空时启用模板替换。
Bun 入口必须显式声明 `transforms`。

Shell 类别不使用 App 的 `metadata_schema_version` 或 Sys 的 `version` 字段。

## Sys 类别 {#sys}

### 类别与项目字段 {#sys-items}

| 类别字段 | 默认值 | 含义 |
| --- | --- | --- |
| `version` | 必填：`2` | Sys 预设格式；缺失或版本 `1` 需要迁移。 |
| `description` | 空字符串 | 类别说明。 |
| `default_profile` | 无 | 默认选择使用的已有组合。存在安装项且非交互 bootstrap 未显式选择项目/组合时必须提供。 |
| `permission_defaults` | 无 | 没有独立权限表的项目使用的[权限](#permissions)。 |
| `items` | `[]` | 安装项或受管设置项目。 |
| `profiles.<name>.items` | `[]` | 已有安装项 ID 的数组，不能包含受管项目；组合不定义 Shell 内容，也不禁用其他集成。 |

每个 `[[items]]` 条目支持：

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `id` | 必填字符串 | 唯一 ID，可使用字母、数字、`-`、`_`。 |
| `label` | 必填字符串 | 非空显示名称。 |
| `description` | 空字符串 | 项目说明。 |
| `default` | `false` | 没有 `default_profile` 时，在交互选择中预选此安装项。 |
| `mode` | `init` | `init` 通过 bootstrap 安装软件；`managed` 应用或移除系统设置。 |
| `requires_admin` | `false` | 声明管理员权限；包管理器也可能独立要求管理员权限。 |
| `required_env` | `[]` | 必需的 Shine 环境变量名，不能重复；缺失或仅含空白的值会导致预检失败。 |
| `permissions` | 类别 `permission_defaults` | 替换此项目的默认权限表。 |
| `detect`、`install` | `init` 必填 | 下文的检测与安装说明；`managed` 禁止使用。 |
| `shell` | `[]` | 下文的 Shell 集成；`managed` 禁止使用。 |
| `driver` | `script` | `init` 使用 `script`；受管资源使用 `split-dns` 或 `managed-file`。 |
| `config` | 空表 | 驱动专用设置；`script` 不接受设置。 |

### 检测与安装 {#sys-install}

`[items.detect]` 必须选择一个 `kind`：

| 类型 | 字段与默认值 |
| --- | --- |
| `command` | 必填 `command` 名称；可选 `version_args = []`，用于向该程序请求版本信息。 |
| `path` | 必填 `path`，检查路径是否存在。 |
| `any` | 必填非空 `probes` 数组，条目为 `{ kind = "command", command = "..." }` 或 `{ kind = "path", path = "..." }`；任一检查成功即可。 |

`[items.install]` 必须选择一个 `kind`：

| 类型 | 字段与默认值 |
| --- | --- |
| `package` | 必填 `provider` 和 `package`。非 Windows 系统可用 `homebrew`/`homebrew-cask`，Ubuntu 使用 `apt`，Arch 使用 `pacman`，Windows 使用 `winget`。填写单个包标识，不填命令选项。 |
| `script` | 必填类别内相对路径 `path`，只安装此项目。 |
| 两者共有 | 可选 `success_status` 默认为 `installed`；`needs-action` 表示用户仍需完成后续步骤。可选 `success_hint` 默认为空字符串。 |

已检测到的软件会跳过，bootstrap 不升级它。Arch 安装前需要完成全系统更新，见
[Arch 准备工作](../built-in-presets/system-bootstrap.md#arch-linux-preparation)。

### Shell 集成：`[[items.shell]]` {#sys-shell}

`shells` 也决定默认列表是否显示项目，以及自动 bootstrap 是否选择项目：任一集成匹配目标 Shell
即可纳入，未声明集成的项目不受限制。显式安装项目会跳过此筛选；`when_command`、phase 和
安装检测不影响选择。这些声明不会扩展平台[支持的 Shell 范围](../guides/system-init.md#平台说明)。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `shells` | 必填非空数组 | `bash`、`zsh` 或 `powershell`。 |
| `phase` | 必填 | `pre` 或 `post`，在用户主要 profile 内容之前或之后加载。 |
| `priority` | `0` | 同一阶段内数字越小越先加载，相同时按声明顺序。 |
| `when_command` | 无 | 仅在该命令可用时应用此集成。 |
| `path` | 无 | 向命令搜索路径添加目录。 |
| `env` | 空表 | 设置字面环境值，不是上文的结构化秘密输入数组。 |
| `eval` | `[]` | 执行参数数组，并将输出作为 Shell 配置求值；`{shell}` 替换为 `bash`、`zsh` 或 `pwsh`。 |
| `source` | 无 | 加载已有 Shell 文件。 |
| `aliases` | 空表 | 为 Bash/Zsh 将别名映射到命令；PowerShell 使用 `fragment`。 |
| `fragment` | 无 | 加载类别内相对路径的 Shell 片段。 |

每项集成必须且只能选择 `path`、`env`、`eval`、`source`、`aliases`、`fragment` 之一。
可执行集成仍需代码授权。

### 受管驱动：`[items.config]` {#sys-managed}

| 驱动 | 配置 |
| --- | --- |
| `split-dns` | 必填字符串 `domain_env` 和 `servers_env`，指定保存 DNS 后缀与服务器地址的 Shine 变量名；两个名称都必须列入 `required_env`。 |
| `managed-file` | 必填 `source` 相对于类别目录；`target` 必须解析为绝对路径，支持 `~/` 和 `$HOME/`。可选 `transforms = []` 使用共用转换流水线；可选 `restart_hint` 显示后续操作说明。 |

受管项目通过 `shine sys apply <ITEM>` 显式应用，不放入 bootstrap 组合。
前置条件、移除与恢复见[系统设置](../guides/system-init.md#受管系统项目)。

## 能力声明 {#permissions}

App 使用 `[permissions]`，Shell 命令使用 `[files.permissions]`，Sys 使用 `[items.permissions]`。
Shell 与 Sys 还支持类别级 `[permission_defaults]`；条目自己的表会整体替换默认表。
权限表可选，但 App 脚本检查识别出的行为必须声明。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `schema_version` | 必填 | `1` 或 `2`；`opaque_code` 要求 `2`。 |
| `administrator` | `false` | 声明管理员权限，不授予权限。 |
| `opaque_code` | 无 | 仅支持 `"unrestricted"`，声明无法完整描述其影响的代码，不绕过 App 代码检查。 |
| `filesystem` | `[]` | 每项必须有 `access`、`base`、`path`。`access` 是由 `read`、`write`、`remove`、`execute` 组成的非空数组；base 可选 `home`、`shine`、`data-dir`、`preset`、`absolute`。 |
| `network` | `[]` | 用 `{ scope = "host", host = "api.example.com" }` 指定主机，或用不含 host 的 `{ scope = "any" }`。host 不能包含 URL 协议、路径、凭据或空白。 |
| `commands` | `[]` | 不带参数的程序名。 |
| `environment` | `[]` | 每项必须有 `name` 和 `sensitivity = "plain"` 或 `"secret"`，不填写值；`env` 输入已经提供对应声明。 |
| `system` | `[]` | 每项必须有 `capability`，可选 `resource` 标识系统设置。能力名以 ASCII 小写字母或数字开头，后续可用这些字符及 `.`、`_`、`-`。 |

除 `absolute` 外，文件系统路径均相对于所选基准目录，不能包含 `..`。相对路径使用 `/` 分隔，
不能有空组件或 `.` 组件；单独的 `.` 表示基准目录本身。
不要重复输入、命令、脚本或目标字段已经表达的能力。

**声明不构成沙箱或授权。** 脚本使用其进程已有的权限。审阅代码和操作计划后，按
[外部代码授权](../guides/custom-presets.md#授权外部代码)完成授权。

完成类别后，运行 `shine preset validate <PATH>` 和 `shine preset lint <PATH>` 验证。
`shine preset schema --format json` 描述报告、测试 fixture 及部分编写契约，并非覆盖所有
`shine.toml` 字段的完整 schema。
