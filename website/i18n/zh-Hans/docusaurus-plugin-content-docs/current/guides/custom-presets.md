---
title: 自定义预设
sidebar_position: 4
---

# 自定义预设

少量个性化优先使用 overlay；需要完全维护一套预设时，再使用外部 `presets_dir`。

两种模式的 fallback 规则不同：

- 使用内置基础来源时，overlay 覆盖同路径内容，其余路径继续来自二进制内嵌预设；
- 完整外部 `presets_dir` 是 App 与 Shell 类别的权威来源，缺少的内容不会悄悄从二进制补齐。

Shine 会输出 `Preset Source`、可选的 `Presets Overlay`，以及外部 Shell 的部署模式，便于在解释
`list`、`update` 或安装结果前确认当前模型。

## 使用 AI 创建预设

Shine 在源码和 crate 包中提供可移植的 Agent Skill：`skills/shine-preset-author/`。通过 AI
客户端原生的 skill 安装器或 skills 目录机制注册该目录，然后直接用自然语言描述需要的 App
配置、Shell 命令、系统引导流程，或要定制的内置类别。Shine 不会探测或修改 Codex、Claude、
Cursor 等客户端的配置。

该 skill 会先确认当前安装的 Shine 支持静态校验，再选择对应的作者参考，使用当前二进制生成
模板，使用 JSON 校验结果，并且只执行隔离 dry-run。它不会 link 或激活类别，也不会运行 hook、
artifact、generator、安装脚本或真实 bootstrap。为提高跨客户端兼容性，skill 指令使用英文，
但提问和结果说明会跟随用户语言。

不使用 AI 客户端时也可以执行同一流程：

```bash
mkdir -p my-presets/app/my-editor
cd my-presets/app/my-editor
shine preset new app
# 添加配置文件并编辑 shine.toml。
shine preset validate . --format json
```

其它类型把 `preset new` 的参数换成 `shell` 或 `sys`。定制内置类别时，进入仓库或 overlay 根目录
并运行 `shine preset copy <kind>/<name>`；命令会创建类型与类别路径。

`preset validate` 也接受仓库根目录、单个类别目录或其中的 `shine.toml`。根目录模式只扫描
`app/`、`shell/`、`sys/` 下的直接类别目录；空仓库会失败。无论当前宿主是什么系统，它都会
检查 macOS、Linux 与 Windows 声明、引用文件和锁定的 Bun 依赖策略；兼容的无 metadata App/Shell
类别会得到 `legacy_metadata` warning。该命令不会加载当前来源或 overlay 设置、初始化配置、
检查更新、写文件、联网或执行预设代码。

默认输出文本；`--format json` 输出 skill 使用的稳定 `schema_version: 1` 报告。校验错误的
退出码为 1，warning 不会导致失败。

## 声明权限

新预设使用权限 schema v1 声明可审查的 capability identity。App 在类别根部使用
`[permissions]`；Shell 的每个 `[[files]]` 命令分别使用 `[files.permissions]`；Sys 的每个
`[[items]]` target 分别使用 `[items.permissions]`。缺少声明时仍保持兼容，但会产生
`missing_permission_declaration` warning；不支持的版本、未知字段、非法 identity 和重复项均为错误。

```toml
[permissions]
schema_version = 1
administrator = true
filesystem = [
  { access = ["read", "write"], base = "home", path = ".config/example" },
  { access = ["execute"], base = "preset", path = "build.ts" },
]
network = [{ scope = "host", host = "api.example.com" }]
commands = ["bun"]
environment = [{ name = "API_TOKEN", sensitivity = "secret" }]
system = [{ capability = "split-dns", resource = "private-domain" }]
```

Filesystem base 只接受 `home`、`shine`、`data-dir`、`preset` 或 `absolute`；非绝对路径必须是
规范化相对路径，`.` 表示所选 base 的根。Command 只能填写一个不带参数的 program identity。
Environment 只填写变量名及 `plain`/`secret` 敏感度，不能填写值或密文。普通 destination、launcher、
receipt 和固定 package provider 已由现有强类型 metadata 约束，不需要重复描述其内部机制。

权限声明不是授权，也不能证明 opaque script 已完整披露行为。外部 App 代码仍需
`allow_app_hooks = true`；外部 Sys 代码仍需全局 `allow_sys_code = true`。安全 Plan 的 planner 与
enforcement 尚未接入，因此当前生命周期执行行为不变。

## 从来源文件夹到已安装能力

任何能把预设文件夹放到机器上的工具或流程，都可以充当同步层。Shine 不依赖 Git，也不承担通用
文件夹同步；它把选定源文件变成已安装能力：创建受管命令入口、解析本地值、默认保留已安装快照、
报告待处理变化，并且只移除自己拥有的内容。

内置 `shell/image-tools/` 类别就是一个完整示例，它通过以下元数据提供三个图片命令：

```toml
description = "Personal image workflow commands."

[[files]]
source = "compress.ts"
target = "img-compress"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY"]

[[files]]
source = "resize.ts"
target = "img-resize"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY", "IMAGE_MAX_WIDTH", "IMAGE_MAX_HEIGHT"]

[[files]]
source = "convert.ts"
target = "img-convert"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY"]
```

这个类别包含三个入口文件和一个共享实现，使用
[`Bun.Image`](https://bun.com/docs/runtime/image) 压缩、缩放和转换 JPEG、PNG、WebP，无需
ImageMagick、Sharp 或其它图片库。运行命令的每台机器都必须在 `PATH` 中安装 Bun 1.3.14 或更高
版本；Shine 不内置 Bun。命令会检测缺失的 `Bun.Image` API，并给出升级提示。

可以安装整个类别或其中一个命令，再沿用普通 Shell 预设的生命周期：

```bash
shine info shell/image-tools
shine install shell/image-tools/img-compress
img-compress photo.jpg screenshots/
img-resize --width 1280 --output-dir ./resized photos/
img-convert --format webp --quality 75 --output-dir ./webp hero.png gallery/
shine info shell/image-tools --diff
shine upgrade shell/image-tools
shine shell uninstall image-tools/img-compress --dry-run
```

每个命令都接受多个文件或目录输入。目录只扫描第一层的 JPEG、PNG、WebP，不会递归。未指定
`--output-dir` 时，结果保存在来源旁边，名称形如 `photo.compressed.jpg`、
`photo.resized.jpg` 或所选转换格式的扩展名。指定输出目录后，所有结果平铺到该目录；重复目标名会
明确失败。已有目标也会失败，只有 `--force` 才允许替换；来源图片永远不会被原地修改。

批处理遇到单项失败后会继续，并在存在任意失败时返回非零状态。前 20 条失败会显示在终端；超过
20 条时，完整列表还会写入 `--output-dir` 下唯一命名的 `image-tools-errors-*.log`，未指定输出目录
时则写入当前目录。

`IMAGE_QUALITY`、`IMAGE_MAX_WIDTH`、`IMAGE_MAX_HEIGHT` 默认分别为 `80`、`1920`、`1080`。
命令参数只覆盖当次运行，本地 Shine 配置则会保留这台机器的长期偏好。默认 snapshot 模式下，修改
复制或外部来源后仍需执行 `shine upgrade`，已安装命令才会变化。这就是“同步一个脚本文件”和“把
脚本作为可复用个人能力运行”之间的边界。

## 使用 Overlay 覆盖少量文件

Overlay 按相同相对路径覆盖基础预设，也可以增加新类别：

```bash
shine preset overlay link ~/dotfiles/shine-overlay --create
shine preset overlay info
shine preset overlay unlink
```

例如 `app/starship/starship.toml` 会覆盖基础来源中的同路径文件，其它预设继续沿用基础来源。

### 使用 Git 镜像 Overlay

要让多台设备使用同一个只读 overlay 仓库，可由 Shine 管理其本地镜像：

```bash
shine preset overlay link --git https://example.com/team/shine-overlay.git --branch main
shine preset pull
shine preset overlay info
```

首次 `shine preset pull` 会在 `~/.shine/overlay/` 浅克隆仓库；以后会把该目录镜像到远端分支的最新状态。此目录是缓存，任何本地修改都会在下次拉取时丢失。请在仓库上游修改并推送，再在设备上运行 `shine preset pull`、`shine update --pull` 或 `shine upgrade --pull` 同步。

如果只想定制一个内置类别，可在 overlay 根目录复制该预设，无需导出整套内容。例如，Surge 的本地代理、策略组和规则文件应从内置预设复制后再修改：

```bash
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
```

命令按 `app/<name>`、`shell/<name>` 或 `sys/<name>` 复制完整类别；已有文件时只有加 `--force` 才会覆盖。复制出的 overlay 仅覆盖保留的相对路径，因此可删除不需要定制的文件，让它们继续使用内置版本；Surge 的后续配置和安装步骤见[管理应用配置](./app-presets.md#surge-uri-订阅)。

## 导出完整预设

```bash
shine preset link ~/dotfiles/shine-presets --create
shine preset export
```

配置外部目录后，`install`、`list` 和 `update` 都从该目录读取。命令输出会显示当前激活的预设来源。

### 选择外部 Shell 的部署方式

外部 Shell 预设默认采用 **snapshot** 模式：安装时，Shine 会把有效类别复制到
`~/.shine/installed/shell/`，之后运行受管副本。编辑来源后先运行 `shine update` 检查，再运行
`shine upgrade` 应用；这让 Shell 脚本与 app 配置一样可以先审阅、再更新。旧版的直接链接安装会在
`update` 中报告，并在 `upgrade` 时迁移。

编写预设、希望反复测试源文件内容时，可在关联来源时显式启用 **live** 模式：

```bash
shine preset link ~/dotfiles/shine-presets --live
```

live 模式下，普通 Shell/Bun 源文件内容在下一次调用时直接生效。声明了 `transforms` 的文件会在每次
调用前原子渲染；渲染失败时该次调用会中止，不会继续运行旧输出。变更 `target`、`runtime`、
`transforms` 或 `env` 等入口元数据时，仍必须执行 `shine upgrade` 重建受管入口。要恢复默认行为，
重新执行不带 `--live` 的 `shine preset link <PATH>`，或执行 `shine preset unlink`。

如果移动了已关联的 overlay 或 live preset 目录，请关联新路径后运行 `shine update`。只要有效的
相对文件集合与字节没有变化，snapshot 部署仍保持最新；live 部署则会显示旧、新来源路径，因为
`shine upgrade` 必须重新指向受管命令入口。该来源迁移会与内容变化分开显示。

### 在外部 Bun 预设中使用锁定依赖

内置 Bun 预设继续保持自包含。外部预设与 overlay 若要使用普通 registry 包，必须把
`package.json` 和 `bun.lock` 一起提交到有效脚本所在的同一物理类别目录：

```text
shell/my-tools/
├── shine.toml
├── package.json
├── bun.lock
├── command.ts
└── shared.ts
```

同一约定也适用于 `app/<category>/` 下采用 Bun 的 artifact、teardown 与 generator 脚本。两个
文件缺一不可；首版还会拒绝任何 `trustedDependencies` 声明。Overlay 只有在自身提供有效脚本时才能
使用自己的依赖声明；仅在 overlay 中加入 package 文件，不会为继承的内置脚本启用依赖。

内置脚本和没有锁文件对的外部脚本都以 `bun --no-install` 运行。具备锁文件对的外部脚本以
`bun --install=fallback` 运行，因此第一次真正执行时可能联网下载缺失包；`list` 与 `info` 不会
下载依赖。Shine 不运行 `bun install`，不复制 `node_modules`，也不拥有 Bun 的全局缓存与 virtual
store；卸载 Shine 或某个预设都不会清理这些共享缓存。

Snapshot Shell 预设的 package 或 lock 变化会显示在 `shine update` 中，并在 `shine upgrade` 后
生效。Live 模式会在下一次命令调用时读取当前文件，同时状态仍会提示刷新安装 receipt。完全离线的
机器需要提前填充相应 Bun 缓存，或由作者 bundle/vendor 脚本。首版不保证原生扩展、workspace、
`file:`、`link:` 以及需要生命周期脚本的依赖可用。

若现有外部脚本依赖 Bun 的隐式安装，请在脚本类别根创建 `package.json`，使用仓库规定的 Bun
版本生成 `bun.lock`，提交两者，并在空 Bun 缓存下测试。没有这对文件时，裸包导入现在会直接失败，
不会再自动下载。

也可以直接设置环境变量：

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine preset export
```

## 建立可提交的预设仓库

```bash
cd ~/dotfiles/shine-presets
shine init
```

该命令创建 `shine.config.toml`，将 `presets_dir` 设为当前目录。Shine 会从工作目录向上查找最近的项目配置，因此可在其子目录运行命令。非交互脚本可使用 `shine init --yes`。

## 拉取 Git 管理的来源

外部预设目录或手动链接的 overlay 是 Git 工作区时，可以只拉取来源，或在检查、应用配置前拉取：

```bash
shine preset pull
shine update --pull
shine upgrade --pull
```

Shine 会定位基础预设和当前 overlay 所在的 Git 仓库；两个来源属于同一仓库时只拉取一次，非 Git 来源会跳过。`update --pull` 和 `upgrade --pull` 会在拉取后重新加载配置，因此仓库中更新的 `shine.config.toml` 也会在后续步骤生效。

拉取前，所有待处理仓库都必须满足以下条件：

- 工作区没有已跟踪或未跟踪的改动；
- 当前处于分支上，而不是 detached HEAD；
- 当前分支已经设置 upstream。

这些来源实际使用 `git pull --ff-only`。Shine 不会自动 stash、rebase、reset 或解决分支冲突；验证失败时会在修改任何仓库前停止。上述限制不适用于 `--git` 管理的 overlay：它设计为可丢弃的远端镜像。Git 不在 `PATH` 中时也无法使用这些命令。

## 新建类别元数据

在 App、Shell 或 Sys 类别目录中生成 `shine.toml` 模板：

```bash
shine preset new app
shine preset new shell
shine preset new sys
```

已有文件时只有加上 `--force` 才会覆盖。类别格式属于预设作者接口，修改后先运行
`shine preset validate . --format json`，再执行对应的隔离安装 `--dry-run`。Shell 安装 dry-run
只解析计划中的命令入口，不会创建文件、链接、manifest、snapshot、渲染文件或 profile 修改。

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

如果两个条目最终指向同一路径，Shine 会在写入前拒绝整个操作。后续 metadata 若移动已受管 source，`shine upgrade` 只会在旧文件未被修改且新目标不存在时迁移；否则保留两端现状并提示用户处理。

## 可选运行时的 Shell 入口

`runtime` 用于选择 Shell 预设命令入口的运行时，而不是新增交互式 shell。未声明时使用原生 `.sh` 或 `.ps1` 入口；当前唯一可选值是 `bun`。请在类别的 `shine.toml` 中显式声明：

```toml
[[files]]
source = "my-tool.ts"
target = "my-tool"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["API_URL", "SERVICE_TOKEN=API_TOKEN"]
```

支持 `.ts`、`.js`、`.mts` 和 `.mjs`。安装后，Shine 会创建无扩展名的受管入口，用户仍以 `my-tool` 调用它；现有 `.sh` 和 `.ps1` 原生入口保持兼容。

运行这类命令的每台设备都必须已在 `PATH` 中安装 Bun。Shine 不会安装 Bun 或管理
`node_modules`；外部类别可以按上文约定启用由 Bun 管理的锁定依赖。`runtime = "bun"` 也不能与
`needs_source = true` 组合使用。

可选的 `env` 只适用于 Bun 入口。每项写成 `KEY` 或 `SOURCE=TARGET`；入口启动时会通过 `shine env run --no-workspace --with ...` 注入值，因此优先解密 `SOURCE_SECRET`，不存在时读取明文 `SOURCE`。声明 `env` 后，运行机器的 `PATH` 中还必须有 `shine`。不要在元数据中填写值或密文，只声明键名。

后续支持的运行时会在本节逐项记录其支持值、文件类型、前提条件和限制。Python、Node 与 Deno 目前均不可配置为 `runtime`。

## 编写系统 Bootstrap Item

一个 sys 类别对应一个操作系统目录，例如 `sys/ubuntu/`。每个可执行 sys 预设都要声明
`version = 2`，再用 detection 和固定 provider 描述普通的 ensure-present 软件：

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
provider = "homebrew" # 也支持 homebrew-cask、apt 或 winget
package = "mise"

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]

[profiles.recommended]
items = ["mise"]
```

Detection 支持 `command`、`path`，以及由 command/path probes 组成的 `any`。包安装是固定的
ensure-present 操作：Shine 负责 argv、提权、代理、超时、输出限制和安装后检测，但不会升级包。
复杂 item 可使用 `[items.install] kind = "script", path = "install/<item>.sh"`；脚本只处理该
item，并以普通 exit code 返回结果。每个 init item 都必须同时声明 `detect` 与 `install`，不会再回退到平台级 dispatcher。version 1 manifest 会在 detection 或 profile 写入前被拒绝；请参阅 [v2 迁移指南](sys-preset-v2-migration.md)。

Shell integration 必须且只能声明 `path`、`env`、`eval`、`source`、`aliases` 或 `fragment` 之一。
`profile/base.pre.sh` 与 `profile/base.post.sh` 只放操作系统公共内容；复杂的 item 逻辑放入
`profile/<item>.sh`。phase、可选 priority、manifest 顺序和声明顺序共同决定稳定的组合顺序。
命名 `[profiles.*]` 表只选择 bootstrap items，不定义 shell 内容，也不会禁用选择之外的集成。

外部 sys 安装脚本和可执行 profile 内容（`eval`、`source`、fragment 与 base 文件）要求用户先审查
来源并在全局配置中设置 `allow_sys_code = true`；项目配置不能授权自身。如果可执行 sys 代码在
bootstrap 预检阶段被拦截，错误会指出可用的代码类型和路径、当前每一层外部 preset 来源以及全局
配置路径，并分别给出“授予权限”和“继续阻止外部代码”两种操作；此时尚未运行任何安装器。静态
detection、package metadata、PATH、env 和 aliases 无需该授权。使用
`shine sys list`、`shine sys info <ITEM>` 和
`shine sys bootstrap <ITEM> --dry-run` 完成验证。

## App 构建脚本运行时

App 类别的 `[artifact]` 也可选择 Bun，使 `build` 与 `unbuild` 脚本跨平台运行：

```toml
[artifact]
script = "build.ts"
teardown = "unbuild.ts"
runtime = "bun"
```

`runtime` 省略时为 `native`，直接执行脚本；`bun` 仅接受 `.ts`、`.js`、`.mts` 或 `.mjs`，并要求运行机器已安装 Bun。构建脚本会收到当前 Shine `[env]` 和 app 路径变量。若希望安装或升级实际改动文件后自动构建，可另外声明 `post_install`、`post_upgrade` 钩子；外部预设仍需用户设置 `allow_app_hooks = true`。
