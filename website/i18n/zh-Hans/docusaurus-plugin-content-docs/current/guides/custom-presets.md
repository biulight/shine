---
title: 自定义预设
sidebar_position: 4
---

# 自定义预设

少量个性化优先使用 overlay；需要完全维护一套预设时，再使用外部 `presets_dir`。

按目标选择起点：

| 需求 | 从这里开始 |
| --- | --- |
| 定制少量内置文件 | [关联 Overlay](#使用-overlay-覆盖少量文件) |
| 维护完整的自有预设集合 | [导出预设](#导出完整预设) |
| 新建 App、Shell 或 Sys 类别 | [新建类别元数据](#新建类别元数据) |
| 让 AI 起草预设 | [使用 AI 创建](#使用-ai-创建预设) |
| 试用已写好的类别 | [预览并安装](#预览并安装自己的类别) |

两种来源的补充规则不同：

- 使用内置基础来源时，overlay 覆盖同路径内容，其余路径继续使用内置预设；
- 完整外部 `presets_dir` 是 App 与 Shell 类别的权威来源，缺少的内容不会从内置预设补齐。

查看命令输出中的 `Preset Source` 和 `Presets Overlay`，确认 Shine 当前使用的文件来源。
请将示例路径和 `<PATH>` 替换为自己的目录，包含空格的路径需要加引号。

## 使用 Overlay 覆盖少量文件

Overlay 按相同相对路径覆盖基础预设，也可以增加新类别：

```bash
shine preset overlay link ~/dotfiles/shine-overlay --create
shine preset overlay info
```

不再使用此 overlay 时再执行：

```bash
shine preset overlay unlink
```

解除链接只移除当前配置中的 overlay 设置，不删除源文件。移除项目覆盖设置后，可能重新继承
全局 overlay；可运行 `shine preset overlay info` 查看之后实际生效的 overlay。

例如 `app/starship/starship.toml` 会覆盖基础来源中的同路径文件，其它预设继续沿用基础来源。

如果只想定制一个内置类别，可在 overlay 根目录复制该预设，无需导出整套内容。例如，Surge 的本地代理、策略组和规则文件应从内置预设复制后再修改：

```bash
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
```

命令按 `app/<name>`、`shell/<name>` 或 `sys/<name>` 复制完整类别；已有文件时只有加 `--force` 才会覆盖。复制出的 overlay 仅覆盖保留的相对路径，因此可删除不需要定制的文件，让它们继续使用内置版本；Surge 的后续配置和安装步骤见[管理应用配置](./app-presets.md#surge-uri-订阅)。

### 使用 Git 镜像 Overlay

要让多台设备使用同一个只读 overlay 仓库，可由 Shine 管理其本地镜像：

Git 托管 overlay 属于全局配置。请在不受任何 `shine.config.toml` 管理的目录中运行以下命令。
如果需要项目专属 overlay，请自行克隆仓库，再在项目中使用 `shine preset overlay link <PATH>`
关联本地目录。

```bash
shine preset overlay link --git https://example.com/team/shine-overlay.git --branch main
shine preset pull
shine preset overlay info
```

请把托管镜像当作只读副本：拉取会替换其内容，包括本地修改和额外文件。请在另一个工作区修改、
推送，再在各设备运行 `shine preset pull`。需要保留本地修改时，使用手动关联的 overlay。
未指定 `--branch` 时，Shine 跟随远端默认分支。如果提示镜像路径被其他目录占用，请将该目录移到
别处后重试。

## 导出完整预设

```bash
shine preset link ~/dotfiles/shine-presets --create
shine preset export
```

导出内容是内置预设加上当前 overlay 的覆盖和新增文件，不包含仅存在于外部基础目录中的文件；
`preset copy` 始终只提取原始内置类别。

`preset copy` 和 `preset export` 都会拒绝作为输出的链接、特殊文件及链接子目录，即使指定
`--force` 也不例外。请选择普通目录树作为输出位置。未指定 `--force` 时，已有普通文件会保留。

配置外部目录后，`install`、`list` 和 `update` 都从该目录读取。命令输出会显示当前激活的预设来源。

也可以通过环境变量为单次命令选择来源：

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine preset export
```

不再使用整套外部预设时，运行 `shine preset unlink`。它只移除来源设置，不删除源文件或卸载
已安装预设。继承的设置或 `SHINE_PRESETS` 仍可能指定其他来源，请再次检查 `Preset Source`。

### 选择外部 Shell 的部署方式

外部 Shell 预设默认采用 **snapshot** 模式：已安装命令持续使用审阅过的副本，直到你应用更新。
编辑来源后，运行 `shine update` 检查变化，再用 `shine upgrade` 应用。

编写预设、希望反复测试源文件内容时，可在关联来源时显式启用 **live** 模式：

```bash
shine preset link ~/dotfiles/shine-presets --live
```

以 live 模式安装后，Shell 和 Bun 源文件的修改在下一次调用时生效；模板替换也会在每次调用时
更新，替换失败就会停止命令。修改 `target`、`runtime`、`transforms` 或 `env` 后，仍需执行
`shine upgrade`。

保留同一来源并恢复 snapshot 模式时，运行不带 `--live` 的
`shine preset link ~/dotfiles/shine-presets`。无论向哪个方向切换，都要对需要切换的已安装类别
运行 `shine update shell/<CATEGORY>`，再运行 `shine upgrade shell/<CATEGORY>`。
只修改关联设置不会改变已安装命令。

Live 模式需要[开发信任](#声明权限)，也就是同时授权未来的代码修改。移动已关联的来源后，请关联
新路径并运行 `shine update`，再用 `shine upgrade` 应用报告中的变更。

### 在外部 Bun 预设中使用锁定依赖

内置 Bun 预设不需要额外安装依赖。外部预设与 overlay 若要使用包仓库中的依赖，必须把
`package.json` 和 `bun.lock` 一起提交到有效脚本所在的同一类别目录：

```text
shell/my-tools/
├── shine.toml
├── package.json
├── bun.lock
├── command.ts
└── shared.ts
```

同一约定也适用于 `app/<category>/` 下采用 Bun 的 artifact、teardown 与 generator 脚本。两个
文件缺一不可；不允许任何 `trustedDependencies` 声明。Overlay 只有在自身提供有效脚本时才能
使用自己的依赖声明；仅在 overlay 中加入 package 文件，不会为继承的内置脚本启用依赖。

缺少所需清单与锁文件的脚本不能自动下载依赖。具备这两个文件的外部脚本首次执行时可能下载
缺失包；`list` 与 `info` 不会下载。分享预设时应带上清单和锁文件，不要依赖本地 `node_modules`
目录被一起带到另一台机器。

Snapshot Shell 预设的 package 或 lock 变化会显示在 `shine update` 中，并在 `shine upgrade` 后
生效。Live 模式会在下一次命令调用时读取当前文件，同时状态仍会提示刷新已安装命令。完全离线的
机器需要提前缓存相应依赖，或由作者将依赖打包进脚本。目前不保证原生扩展、workspace、
`file:`、`link:` 以及需要生命周期脚本的依赖可用。

使用预设所支持的 Bun 版本生成并提交 `bun.lock`。分享预设前，请在空 Bun 缓存下测试依赖加载。

## 建立可提交的预设仓库

```bash
cd ~/dotfiles/shine-presets
shine init
```

该命令创建 `shine.config.toml`，将 `presets_dir` 设为当前目录。Shine 会从工作目录向上查找最近的项目配置，因此可在其子目录运行命令。非交互脚本可使用 `shine init --yes`。

## 拉取 Git 管理的来源

外部预设目录或手动链接的 overlay 是 Git 工作区时，可以只拉取来源，或在检查、应用配置前拉取：

按需选择一种拉取或更新操作：

| 操作 | 命令 |
| --- | --- |
| 仅拉取预设源 | `shine preset pull` |
| 拉取后检查更新 | `shine update --pull` |
| 拉取后更新配置 | `shine upgrade --pull` |

请先确保 `PATH` 中有 Git，并提交或另行保存本地修改。拉取到的 `shine.config.toml` 变更也会
影响后续 `update` 或 `upgrade`。

拉取前，所有待处理仓库都必须满足以下条件：

- 工作区没有已跟踪或未跟踪的改动；
- 当前处于分支上，而不是 detached HEAD；
- 当前分支已经设置 upstream。

这些来源使用 `git pull --ff-only`。Shine 不会自动 stash、rebase、reset 或解决分支冲突；请自行
处理报告中的工作区或分支问题后重试。上述限制不适用于 `--git` 管理的 overlay，其本地镜像会被拉取替换。

## 新建类别元数据

每个类别都有自己的目录和 `shine.toml`。例如，新建一个 App 类别：

```bash
mkdir -p ~/dotfiles/shine-overlay/app/my-editor
cd ~/dotfiles/shine-overlay/app/my-editor
shine preset new app
```

这只会创建元数据。请在同一目录添加 `config.toml`，并将 `shine.toml` 中的 `dest` 改为应用的
配置目录，保留生成的 `metadata_schema_version = 2`。

创建其他类型时，改为在 `shell/<name>` 或 `sys/<os>` 下建立目录，并在其中执行对应命令：

| 操作 | 命令 |
| --- | --- |
| 创建 App 模板 | `shine preset new app` |
| 创建 Shell 模板 | `shine preset new shell` |
| 创建 Sys 模板 | `shine preset new sys` |

若从内置类别开始，请在仓库或 overlay 根目录运行 `shine preset copy <kind>/<name>`，命令会创建
对应的类型与类别路径。

已有文件时只有加上 `--force` 才会覆盖，请先检查现有内容。添加 `shine.toml` 引用的配置文件或
脚本，并按[下文](#声明权限)声明脚本访问和必需输入，然后[验证该类别](#验证与打包预设)，
再[预览安装](#预览并安装自己的类别)。

### 为单个 App 文件指定目标目录

App 类别的顶层 `dest` 是默认目标根目录；显式 `[[files]]` 条目可以用自己的 `dest` 覆盖它，`target` 仍然是相对于最终根目录的路径：

```toml
dest = "~/.config/my-app"

[[files]]
source = "config.toml"
target = "config.toml"

[[files]]
source = "shared/rules.list"
target = "rules/provider.list"
dest = { base = "data-dir", path = "com.example.my-app" }
```

文件级覆盖既支持类别已有的绝对路径字符串，也支持平台映射。映射可使用精确的 `macos`、`linux`、`windows` 键，并可用 `unix` 作为 macOS/Linux 回退；同时存在时精确键优先。某个平台缺少分支时，该类别或文件不会在对应系统出现。App 与 Shell 显式文件的 `platforms` 数组使用同样四个选择器，按 OR 语义组合且不能为空。仅文件级可使用结构化 `data-dir`：它在 Windows 解析为 `%APPDATA%`，在 macOS 解析为 Application Support，在 Linux 解析为 `XDG_DATA_HOME`（未设置时为 `~/.local/share`）。`path` 和 `target` 必须是相对路径，且不能包含 `..`。

没有 `shine.toml` 的旧 App 预设仍支持在首行或 shebang 的下一行用 `# shine-dest:` 指定完整目标路径。
展开主目录后，路径必须是绝对路径，且不能包含 `..`。请改用不含父目录组件的明确目标路径后再重试。

如果两个条目最终指向同一路径，Shine 会在写入前拒绝整个操作。后续修改元数据、移动已安装文件的
目标位置时，`shine upgrade` 只会在旧文件未被修改且新目标不存在时迁移；否则保留两端现状并提示处理。

## 使用 AI 创建预设

Shine 在源码仓库中提供 Agent Skill：`skills/shine-preset-author/`。通过 AI 客户端的 skill
安装器或 skills 目录安装后，直接描述需要的 App 配置、Shell 命令、系统初始化流程，或要定制的内置类别。

该 skill 会生成预设、完成校验，并在隔离目录中预览安装，不会激活预设或执行其中的脚本。
安装前请审阅结果。

## 参考内置示例 {#从来源文件夹到已安装能力}

需要完整的 Shell 示例时，可将内置图片工具复制到 overlay：

```bash
cd ~/dotfiles/shine-overlay
shine preset copy shell/image-tools
```

结合脚本阅读 `shell/image-tools/shine.toml`，了解如何配置命令名、Bun、支持的平台和环境输入。
命令用法见[图片工作流](../reference/built-in-presets.md#image-workflow)。

## 声明运行时环境输入

Shell Bun 命令与 App 的生成器（generator）、钩子（hook）和构建脚本（artifact）使用结构化 `env`：

```toml
env = [
  { source = "API_URL", sensitivity = "plain" },
  { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" },
]
```

`source` 是 Shine 配置中的变量名，`target` 是脚本读取的名称，省略时与 `source` 相同。
这里只声明名称和敏感度，不填写值或密文；实际值通过[环境变量配置](./environment.md)另行设置。
每个入口只接收自身声明的 Shine 输入。同一来源的敏感度声明应保持一致；
`sensitivity = "secret"` 本身不会加密变量。生成器的 `when_env` 必须填写其 `env` 列表中的来源名称。

旧的 `env = ["SOURCE=TARGET"]` 字符串数组已不再支持。请按[元数据迁移](#迁移-1x-来源)转换，
并检查缺失或冲突的敏感度声明。命令行 `shine env run --with SOURCE=TARGET` 语法不变。

## 可选运行时的 Shell 入口

`runtime` 用于选择 Shell 预设命令入口的运行时，而不是新增交互式 shell。未声明时使用原生 `.sh` 或 `.ps1` 入口；当前唯一可选值是 `bun`。请在类别的 `shine.toml` 中显式声明：

```toml
[[files]]
source = "my-tool.ts"
target = "my-tool"
runtime = "bun"
platforms = ["unix", "windows"]
env = [{ source = "API_URL", sensitivity = "plain" }, { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" }]
```

支持 `.ts`、`.js`、`.mts` 和 `.mjs`。安装后使用 `my-tool` 调用，不需要加源文件扩展名。

运行这类命令的每台设备都必须已在 `PATH` 中安装 Bun。Shine 不会安装 Bun 或管理
`node_modules`；外部类别可以按上文约定启用由 Bun 管理的锁定依赖。`runtime = "bun"` 也不能与
`needs_source = true` 组合使用。

可选的 `env` 只适用于 Bun 入口，遵循[环境输入格式](#声明运行时环境输入)。每个已声明来源优先读取
`SOURCE_SECRET`，其次读取明文 `SOURCE`，不会额外加入工作区的默认秘密绑定。声明 `env` 后，
运行机器的 `PATH` 中还必须有 `shine`。元数据只声明名称与敏感性，不填写值或密文。

Python、Node 与 Deno 目前均不可配置为 `runtime`。

## 编写系统 Bootstrap Item

一个 Sys 类别对应一个操作系统目录，例如 `sys/macos/`。下面以 macOS 的 Homebrew 为例：
声明 `version = 2` 后，逐项描述如何检测和安装软件。

```toml
version = 2

[[items]]
id = "mise"
label = "mise"
description = "Install mise without managing its versions."

[items.detect]
kind = "command"
command = "mise"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew" # 也支持 homebrew-cask、apt、pacman 或 winget
package = "mise"

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]

[profiles.recommended]
items = ["mise"]
```

`provider = "pacman"` 仅适用于 `sys/arch/`，每个 item 只声明一个包名。它使用已有的
pacman 数据库；bootstrap 前应先完成全系统更新，见 [Arch Linux 准备工作](system-init.md#arch-linux-preparation)。

每个安装项都需要 `detect` 和 `install`。检测可使用 `command`（命令）、`path`（路径），或用
`any` 组合多项命令与路径检查。包安装只确保软件存在，不升级已有安装。需要自定义安装逻辑时，
在 `[items.install]` 中设置 `kind = "script"` 和 `path = "install/<item>.sh"`，脚本只处理
自己的安装项。旧版预设请按 [v2 迁移指南](sys-preset-v2-migration.md)转换。

每项 Shell 集成只能声明 `path`、`env`、`eval`、`source`、`aliases` 或 `fragment` 之一。
用 `phase = "pre"` 或 `"post"` 选择加载阶段；同一阶段内，`priority` 越小越先加载，相同时按
`shine.toml` 中的声明顺序加载。`profile/base.pre.sh` 与 `profile/base.post.sh` 只放操作系统
公共内容，复杂的安装项逻辑放入 `profile/<item>.sh`。`[profiles.*]` 表只选择初始化安装项，
不定义 Shell 内容，也不会禁用选择之外的集成。

外部 Sys 安装脚本和可执行 Shell 配置需要[代码授权](#声明权限)。关联来源后，使用
`shine sys list` 和 `shine sys info <ITEM>` 检查，再用 `shine sys bootstrap <ITEM> --dry-run`
预览后决定是否应用。

## App 构建脚本运行时

App 类别的 `[artifact]` 也可选择 Bun，使 `build` 与 `unbuild` 脚本跨平台运行：

```toml
[artifact]
script = "build.ts"
teardown = "unbuild.ts"
runtime = "bun"
env = [{ source = "PROFILE_PATH", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }]
```

`runtime` 省略时为 `native`，直接执行脚本；`bun` 仅接受 `.ts`、`.js`、`.mts` 或 `.mjs`，并要求
运行机器已安装 Bun。只有 `env` 列出的 Shine `[env]` 变量会传给构建脚本；每项同时声明来源、
可选目标别名与敏感度，无需在 `[permissions].environment` 重复声明。未配置的可选值会被省略；
固定 App 路径变量见下文。
外部预设需要人工确认操作计划，或已有匹配的信任授权。

运行构建脚本时，使用 `shine app artifact apply <CATEGORY>` 并审阅计划。完整操作见
[应用构建脚本](./tasks-and-serve.md#生成-app-辅助资源)。

### App 配置改动后运行脚本

在 `shine.toml` 顶层、任何表头之前声明 `post_install` 或 `post_upgrade`，可填写单个 hook 或列表。
例如：

```toml
post_upgrade = [{ script = "refresh.ts", runtime = "bun", env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }] }]

[permissions]
schema_version = 1
network = [{ scope = "host", host = "api.example.com" }]
```

每个 hook 必须且只能选择 `command` 或 `script`。命令 hook 运行程序，可带 `args`；脚本 hook
运行类别内的相对路径脚本，可指定 `runtime = "bun"`。`runtime` 只能与 `script` 一起使用。
Bun 文件扩展名和依赖规则与 artifact 相同。请声明脚本实际需要的访问；此示例假定请求
`api.example.com`。

命令 hook 的已声明输入必须有配置值；脚本 hook 会省略缺失的可选输入，脚本应自行处理。
必需输入缺失时会阻止批准。外部 hook 与其他 App 代码一样需要审阅或信任授权。
不要从 hook 调用 artifact 生命周期命令，应使用显式 artifact 操作。触发时机和失败处理见
[生命周期 hook](./app-presets.md#生命周期钩子)。

### App 脚本可用的路径

Generator、脚本 hook、artifact 与 teardown 无需 `env` 声明即可读取以下变量；命令 hook
不会获得这组 App 专用变量。

| 变量 | 用途 |
| --- | --- |
| `SHINE_APP_ID` | 应用类别名称 |
| `SHINE_APP_DIR`、`SHINE_APP_SOURCE_DIR` | 本次脚本使用的文件目录，也是工作目录 |
| `SHINE_APP_OVERLAY_DIR` | 本次运行的 overlay 文件目录；没有 overlay 时为空 |
| `SHINE_CONFIG_DIR` | 当前 Shine 配置与状态根目录 |
| `SHINE_STATE_DIR` | 当前应用类别的持久输出 |
| `SHINE_CACHE_DIR` | 当前应用类别的缓存输出 |

脚本来源目录是临时目录，请勿保存其路径或向其中写入持久结果。需要保留的输出使用
`SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`，必要时由脚本创建输出目录。

## 说明能力和必需输入 {#声明权限}

在 `shine.toml` 中说明脚本需要的访问：App 类别使用 `[permissions]`，Shell 命令使用
`[files.permissions]`，Sys 安装项使用 `[items.permissions]`。权限表本身是可选的，但 App
代码检查识别出的行为必须有相应声明。

例如：

```toml
[permissions]
schema_version = 1
filesystem = [
  { access = ["read", "write"], base = "home", path = ".config/example" },
]
network = [{ scope = "host", host = "api.example.com" }]
```

Shell 和 Sys 类别可以用 `permission_defaults` 为没有目标级覆盖的条目提供同一声明：

```toml
[permission_defaults]
schema_version = 1
network = [{ scope = "host", host = "api.example.com" }]

[[files]]
source = "foo.sh"
target = "foo"

[[files]]
source = "bar.sh"
target = "bar"
```

条目自己的权限表会替换该条目的 `permission_defaults`。

文件系统基准目录可选 `home`、`shine`、`data-dir`、`preset` 或 `absolute`。除 `absolute` 外，
路径必须为相对路径且不能包含 `..`，`.` 表示基准目录本身。命令只填写不带参数的程序名；环境声明只填写变量名及
`plain`/`secret` 敏感度，不填写值。不要重复其他元数据字段已经声明的输入、命令、脚本路径或目标路径。

**脚本没有沙箱隔离**，可使用其进程已有的权限。权限说明（包括 schema v2 的
`opaque_code = "unrestricted"`）不会授予信任或管理员权限。请显式声明环境输入；需要管理员权限时，
添加 `administrator = true`。

### 授权外部代码

交互式安装或更新时，请审阅操作计划并确认其中列出的代码。本次授权仅适用于当前操作和已审阅内容，
也包含安装后才运行的 Shell 命令。审阅后内容变化需要重新批准。`--yes` 不能代替人工确认；
自动化操作必须事先获得匹配的信任授权。

需要持久授权时，先检查目标，再选择所需范围：

| 操作 | 命令 |
| --- | --- |
| 检查代码及所需访问 | `shine trust inspect <TARGET>` |
| 信任已审阅的版本 | `shine trust grant <TARGET>` |
| 同时信任未来的代码修改 | `shine trust grant <TARGET> --development` |
| 查看授权 | `shine trust list` |
| 撤销授权 | `shine trust revoke <TARGET>` |

目标写作 `app/<CATEGORY>`、`shell/<CATEGORY>/<COMMAND>` 或 `sys/<ITEM>`。
`preset` 代表当前预设中的所有可执行目标；
`shine trust revoke preset` 会清除所有已保存的预设信任授权。

快照信任在类别文件、元数据、来源层或可执行权限变化后需要重新审阅；它不会验证外部依赖、解释器、
工具或下载内容。开发信任还会接受所选本地来源中的未来代码修改；移动来源、新增目标或所需能力都
需要重新授权。仅在接受这一范围时使用开发信任；Shell 的 live 模式必须使用它。希望逐次审阅更新时，
请选择 snapshot 模式。

显式 `info`/`update --run-generators` 同样需要持久信任；若只想授权一次生成操作，可使用交互式
`shine app refresh`。旧授权若提示需要重新审阅，请检查目标后重新授权。

AI 生成的代码也需要你审阅。对 AI 可写目录授予开发信任，也会授权所覆盖目标将来的 AI 修改。

## 验证与打包预设

在类别目录中依次执行：

```bash
shine preset validate .
shine preset lint .
shine preset plan . --platform macos
```

`validate` 检查元数据和引用文件，也接受预设仓库根目录或 `shine.toml` 文件。无论当前运行什么
系统，`validate` 和 `lint` 都会检查受支持的平台声明。`lint` 检查可移植性和代码声明问题；
请先修复错误，并审阅警告。若 lint 无法加载类别，使用 `validate` 查找输入错误。

对每个需要支持的平台分别运行 `plan`：`macos`、`linux` 或 `windows`。它会预览一次假设的首次
安装，列出所需权限及阻止安装的问题。预览使用模拟条件，因此报告中的输入或信任缺失不代表本机
的实际状态；检查实际安装环境应使用安装命令的 `--dry-run`。这些检查不会执行
预设脚本或修改已安装配置；预览成功也不代表已经批准安装。

需要重复测试时，在类别中加入 `shine.test.toml`，然后运行 `shine preset test .`。测试描述初始
条件和预期结果，不会执行脚本。源码仓库的 `examples/presets/` 目录提供 App、Shell 和 Sys 示例。
需要在其他机器上运行的测试应使用相对路径及 `${HOME}` 等受支持的占位符。
`plan` 和 `test` 每次只接受一个类别。

分发已审阅的类别时，将压缩包保存到类别目录之外：

```bash
shine preset pack . --output ../../my-editor.shine-preset.tar.gz
```

压缩包不包含 `shine.test.toml`。`--force` 只允许替换已有输出，不能绕过校验，也不能将输出放到
类别目录内，包括通过符号链接指向该目录的情况。

## 执行前检查 App 代码

分发 App 脚本前，运行：

```bash
shine preset lint <PATH>
```

Shine 会检查 Bun generator、脚本 hook、artifact、teardown，以及受支持的本地 JavaScript/TypeScript
依赖。根据报告的文件和行号修复阻断问题：声明脚本实际需要的访问，或修改脚本使其符合声明。
源码解析失败也会阻止执行。信任授权、`--yes` 和 `opaque_code = "unrestricted"` 都不能绕过这些检查。

常见声明如下：

| 脚本行为 | 所需声明 |
| --- | --- |
| 请求 `https://example.org/data` | 网络主机 `example.org` |
| 用 `Bun.file('in').text()` 读取 `in` | 对 `in` 的文件系统 `read` |
| 用 Node `copyFile` 复制文件 | 对来源的 `read` 和对目标的 `write` |
| 用 Node `rename` 重命名文件 | 对来源的 `remove` 和对目标的 `write` |
| 读取配置的环境值 | 含 `source`、可选 `target` 和 `sensitivity` 的 `env` 项 |

脚本读取环境映射中的 `target` 名称。只有文档列出的固定变量可以免于声明，`SHINE_` 前缀本身不代表
豁免。不要把环境值或 URL 凭据写入能力声明。

即使 `plan` 或 `test` 通过，也应运行 `lint`：前两者只检查模拟输入选中的脚本，lint 则会检查
所有 App 脚本入口，包括未启用的生成器。

静态检查无法覆盖脚本的所有行为。通过 lint 不代表代码安全；运行前仍需审阅代码，并完成正常的授权步骤。

## 预览并安装自己的类别

验证通过后，关联包含 `app/`、`shell/` 或 `sys/` 的根目录，而不是类别目录本身。
以上面的 `my-editor` 为例：

```bash
shine preset overlay link ~/dotfiles/shine-overlay
shine app info my-editor
shine app install my-editor --dry-run
```

检查输出中的来源、目标位置、必需输入和权限。准备好后，再安装并确认计划：

```bash
shine app install my-editor
```

修改已安装预设后，用 `shine update app/my-editor` 查看变化，再用 `shine upgrade app/my-editor`
应用。解除来源关联不会卸载预设；要移除它，先运行 `shine app uninstall my-editor --dry-run`
预览，再去掉 `--dry-run` 执行。

Shell 类别先运行 `shine shell install <CATEGORY> --dry-run`，再去掉 `--dry-run` 安装，并打开
新终端。Sys 安装项先用 `shine sys bootstrap <ITEM> --dry-run` 预览，再执行实际操作。
前提条件、更新、移除和恢复限制分别见 [Shell 指南](./shell-presets.md)、[App 指南](./app-presets.md)
和[系统指南](./system-init.md)。

## 工具兼容性 {#tool-compatibility}

编辑器和 CI 可为预设编写命令添加 `--format json`。校验错误会使命令失败，警告不会。
如果希望已审阅的警告也导致 CI 失败，可使用 `preset lint --deny-warnings`。

运行 `shine preset schema --format json`，获取当前安装版本支持的报告及 `shine.test.toml` 格式。
升级 Shine 后，请检查工具是否仍兼容这些格式。`complete = false` 或非空的 `unfinished_cases`
表示检查未完成；请解决报告中的问题后再接受结果。

## 迁移 1.x 来源

可预览当前使用的外部来源与 overlay，也可以指定仓库、类别或 `shine.toml`：

| 操作 | 命令 |
| --- | --- |
| 预览当前外部预设源的迁移 | `shine preset migrate --dry-run` |
| 预览指定预设源的迁移 | `shine preset migrate ./my-presets --dry-run` |
| 审阅预览后应用迁移 | `shine preset migrate ./my-presets` |

确认前请审阅元数据差异，默认回答为 No。迁移仅修改 `shine.toml`，并在写入前备份；不会修改
脚本、数据文件、环境值、已安装资源或信任。提示需要手动迁移的条目须自行处理；
旧版 Sys 预设请按 [v2 迁移指南](./sys-preset-v2-migration.md)操作。

仅在审阅后使用 `--yes`。自动化检查可用 `--format json --dry-run`；以 JSON 输出执行迁移时
还须添加 `--yes`。JSON 报告不含文件内容或差异。

Git 管理的 overlay 为只读镜像。请在上游工作区中迁移并验证，提交、推送后，
在各使用设备上运行 `shine preset pull`。

若迁移提示文件混用 LF/CRLF 换行，请为该文件统一换行方式后重试。

迁移后请验证类别，再通过正常的安装或升级命令应用。外部代码变化后，请按提示重新审阅。
