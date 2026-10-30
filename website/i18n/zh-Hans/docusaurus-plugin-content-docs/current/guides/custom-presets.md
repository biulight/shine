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

该 skill 会生成预设、完成校验，并在隔离目录中预览安装，不会激活预设或执行其中的脚本。
安装前请审阅结果。

不使用 AI 客户端时也可以执行同一流程：

```bash
mkdir -p my-presets/app/my-editor
cd my-presets/app/my-editor
shine preset new app
# 添加配置文件并编辑 shine.toml。
shine preset validate . --format json
shine preset lint . --format json
shine preset plan . --platform macos --format json
shine preset test . --format json
```

默认脚手架不会添加空权限表。以下选项会添加 unrestricted 作者说明：

```bash
shine preset new app --unrestricted
shine preset new shell --unrestricted
shine preset new sys --unrestricted
```

无论是否使用该选项，所有任意代码都未隔离。该选项不会跳过 trust、安全 Plan、管理员授权、
所有权检查或显式环境输入。

其它类型把 `preset new` 的参数换成 `shell` 或 `sys`。定制内置类别时，进入仓库或 overlay 根目录
并运行 `shine preset copy <kind>/<name>`；命令会创建类型与类别路径。
`preset validate` 也接受仓库根目录、单个类别目录或其中的 `shine.toml`。根目录模式只扫描
`app/`、`shell/`、`sys/` 下的直接类别目录；空仓库会失败。无论当前宿主是什么系统，它都会
检查 macOS、Linux 与 Windows 声明、引用文件和锁定的 Bun 依赖策略；兼容的无 metadata App/Shell
类别会得到 `legacy_metadata` warning。该命令不会加载当前来源或 overlay 设置、初始化配置、
检查更新、写文件、联网或执行预设代码。

默认输出文本；工具和 CI 可以使用 `--format json`。校验错误的退出码为 1，warning 不会导致失败。
诊断代码表示未通过的检查，不受类别名称或消息措辞影响。例如，引用文件缺失时报告
`missing_reference`，Bun 依赖声明不符合策略时报告 `bun_dependency_policy`。

创作工具需要当前安装版本支持的精确报告与 fixture 格式时，运行
`shine preset schema --format json`。App、Shell 和 Sys metadata 是否可用仍以
`preset validate` 的结果为准。

校验后运行 `preset lint`，检查作者质量和可移植性。warning 默认只是建议；CI 可在审阅后
添加 `--deny-warnings`。
lint 无法加载输入时，请运行 `shine preset validate <PATH>` 查看原因。
Lint 会检查每个平台适用的资源，包括 Windows 的 PowerShell 命令；明确不适用于某个操作系统的
App 类别或文件会在该操作系统的检查中跳过。

然后对每个目标平台运行 `preset plan`。它会预览一次假设的首次安装，显示操作、权限和 blocker，
但不会修改机器。blocker 通常表示预览缺少环境变量、信任决定、命令或管理员状态；这属于作者反馈，
不是实际安装批准。

需要可重复的跨平台预期时，在类别中加入 `shine.test.toml`。测试描述初始条件和预期结果，不会执行
setup code。`examples/presets` 下提供 App、Shell、Sys 三类可运行示例。`preset test` 只接受单个类别，
不能传仓库根目录。
编写跨平台测试时，使用文件的相对路径基准，以及 fixture receipt 中的 `${HOME}` 等占位符。

需要可分发 artifact 时，把已审查类别打包到 source tree 外部：

```bash
shine preset pack . --output ../../my-editor.shine-preset.tar.gz --format json
```

`shine.test.toml` 只供作者测试，不会打包。输出必须位于类别目录之外，符号链接解析后也须满足
此要求。`--force` 只允许替换已有输出，不能绕过校验或打包限制。

## 执行前检查 App 代码

Shine 会自动检查 App 的 Bun generator、脚本 hook、artifact 和 teardown，以及能够解析的本地
JavaScript/TypeScript 依赖。源码行为与声明存在确定差异，或受支持的源码解析失败时，会阻止执行。
根据报告的文件和位置修复：准确声明预期行为，或修改脚本使其符合声明。trust、`--yes` 和
`opaque_code = "unrestricted"` 都不能绕过此检查。

使用 `shine preset lint <PATH>` 检查所有 App 代码入口，包括当前缺少触发输入的 generator。
`preset plan` 和 `preset test` 只检查假设输入下选中的入口，并在报告中显示范围；跳过的入口不算匹配。
这些检查不会执行脚本、下载依赖或读取真实环境值。

动态目标、包依赖、子进程效果和不支持的语言等覆盖缺口只作提示；未被源码印证的声明也不阻断，
可在详细审阅中查看。这些结果都不代表代码安全或已被完整分析。没有诊断时，常规审阅不增加分析
输出；原有 trust 和批准检查仍然适用。

使用 `--verbose` 查看详细审阅，或用 `shine preset lint <PATH> --format json` 获取全部诊断。
`--deny-warnings` 可让 lint 告警使 CI 失败。详细输出不会执行生成器；执行仍需显式使用
`--run-generators` 并具有信任授权。Lint 检查所有受支持入口，可能报告某次运行检查范围之外的问题。

使用结构化 `env` 映射时，在同一项中声明 `source`、可选 `target` 与 `plain`/`secret` 敏感性；脚本读取目标名。
只有文档列出的固定变量可以免于重复声明，任意 `SHINE_` 前缀名称不能获得豁免。不要把环境值或
URL 凭据写入能力声明。源码位置采用从 1 开始的行号和 UTF-8 字节列号。

工具集成在升级后应重新读取 `preset schema`：lint 报告使用 schema 2，作者 Plan 报告使用 schema 3，
fixture 报告使用 schema 2。Fixture 输入仍为 schema 1；可用 `expect.analysis_diagnostic_codes`
配合 `ready` 断言诊断。正确预期阻断的用例可以通过，但取消或命令累计预算耗尽不能成为成功的负向
测试。`complete = false` 表示作者报告未完成，fixture 报告还会列出 `unfinished_cases`。
生命周期 Plan／批准／审阅契约使用 schema 4；应重新审阅，不能复用旧批准。

## 迁移 1.x 来源

可预览当前激活的 external source 与 overlay，也可以指定仓库、类别或 manifest：

```bash
shine preset migrate --dry-run
shine preset migrate ./my-presets --dry-run
shine preset migrate ./my-presets
```

确认前请审阅元数据差异，默认回答为 No。迁移仅修改 `shine.toml`，并在写入前备份；不会修改
脚本、数据文件、环境值、已安装资源或信任。提示需要手动迁移的条目须自行处理；
Sys v1 dispatcher 请按 [v2 迁移指南](./sys-preset-v2-migration.md)操作。

仅在审阅后使用 `--yes`。自动化检查可用 `--format json --dry-run`；以 JSON 输出执行迁移时
还须添加 `--yes`。JSON 报告不含文件内容或差异。

Git 管理的 overlay 为只读镜像。请在上游 checkout 中迁移并验证，提交、推送后，
在各使用设备上运行 `shine preset pull`。

若迁移提示文件混用 LF/CRLF 换行，请为该文件统一换行方式后重试。

## 说明能力和必需输入 {#声明权限}

权限 schema v1/v2 表是可选的作者能力说明。App 在类别根部使用 `[permissions]`；Shell 的每个
`[[files]]` 命令可使用 `[files.permissions]`；Sys 的每个 `[[items]]` target 可使用
`[items.permissions]`。Shine 会校验任何显式提供的表，但缺表或空表不会隐藏或约束任意代码。
受支持的 App 代码仍须准确声明静态分析识别出的行为。不支持的版本、未知字段、非法 identity 和重复项均为错误。

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

权限 schema v2 支持 unrestricted 作者说明：

```toml
[permissions]
schema_version = 2
opaque_code = "unrestricted"
```

Shell 和 Sys 类别可以用 `permission_defaults` 为没有目标级覆盖的条目提供同一声明：

```toml
[permission_defaults]
schema_version = 2
opaque_code = "unrestricted"

[[files]]
source = "foo.sh"
target = "foo"

[[files]]
source = "bar.sh"
target = "bar"
```

默认值会分别解析到每个 command 或 Sys item；定向操作不会继承无关条目的 capability。条目自己的
权限表会替换该条目的默认值。

Filesystem base 只接受 `home`、`shine`、`data-dir`、`preset` 或 `absolute`；非绝对路径必须是
规范化相对路径，`.` 表示所选 base 的根。Command 只能填写一个不带参数的 program identity。
Environment 只填写变量名及 `plain`/`secret` 敏感度，不能填写值或密文。不要重复预设配置中
已声明的命令、脚本路径、普通 destination 或固定 package provider。App 脚本中被静态分析识别的
行为仍须声明；其他说明可用于帮助审阅者理解预期效果。

`opaque_code = "unrestricted"` 用于说明代码供审阅，不会授予信任或管理员权限。
无论是否声明此字段，脚本都可使用其进程已有权限，且没有沙箱隔离。请显式声明环境输入和
管理员要求；能力说明不能证明脚本已完整披露所有影响。

交互式生命周期操作可在用户确认 Plan 时，一并授权其中列出的外部代码。本次授权仅适用于当前
操作和捕获快照，不保存持久 grant；确认也包含安装 Shell 命令等“交付后运行”的代码。
自动化和 `--yes` 必须已有匹配 grant；`--yes` 不能代替人工代码确认。
显式 `info`/`update --run-generators` 检查同样要求 grant；若只想授权一次 generator 操作，
使用交互式 `shine app refresh`。

需要持久授权时，先检查再登记目标：

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust grant <TARGET> --development
```

Snapshot trust 适用于已审阅的目标及有效类别内容。文件、元数据、来源层或可执行权限变化后，
需要重新审阅。该授权不保证 `node_modules` 等依赖、解释器、外部工具及下载内容的安全。

Development trust 是对所显示本地来源中、指定 target/capability 的未来代码的长期授权，
不代表持续代码审阅。来源目录/层变化、新 target 或新 capability 需要重新授权。
Shell live 部署必须使用此模式。live 部署被阻止时，请先运行 `shine trust inspect <TARGET>`，
再决定是否执行 `shine trust grant <TARGET> --development`。不愿授权未来代码修改时，
可设置 `external_shell_mode = "snapshot"` 后重新审阅；普通 snapshot trust 不能授权 live 部署。
`opaque_code = "unrestricted"` 不会启用开发信任。`trust list`、
`trust inspect` 与安全 Plan 都会标注当前有效的开发信任。Grant 只保存在本机私有 trust
store 中，Preset 不能随自身分发或启用它。
使用 `trust list` 检查授权，添加 `--verbose` 查看详情及复查指引。

每个外部 Shell command 都采用规范 target `shell/<CATEGORY>/<COMMAND>`。使用 `preset` 可以一次审阅或信任当前激活 Preset 快照中的全部可执行
target。撤销 `preset` 会清除所有已保存的
Preset trust grant：

```bash
shine trust inspect preset
shine trust grant preset
shine trust grant preset --development
shine trust revoke preset
```

旧授权若提示需要重新审阅，就不能批准新操作。请审阅后重新授权。

AI 工具可生成和静态验证 Preset，但不能批准自己生成的代码。用户须通过可信界面审阅实际变更；
快照修改会使原审批失效。对 AI 可写目录授予 Development trust，意味着允许已登记目标采用以后
的 AI 修改。拥有不受限终端权限的 AI 可以自行 grant 或直接运行脚本，其授权边界必须由工具宿主执行。

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
env = [{ source = "IMAGE_QUALITY", sensitivity = "plain" }]

[[files]]
source = "resize.ts"
target = "img-resize"
runtime = "bun"
platforms = ["unix", "windows"]
env = [{ source = "IMAGE_QUALITY", sensitivity = "plain" }, { source = "IMAGE_MAX_WIDTH", sensitivity = "plain" }, { source = "IMAGE_MAX_HEIGHT", sensitivity = "plain" }]

[[files]]
source = "convert.ts"
target = "img-convert"
runtime = "bun"
platforms = ["unix", "windows"]
env = [{ source = "IMAGE_QUALITY", sensitivity = "plain" }]
```

这些命令使用
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
冲突检查涵盖仅大小写或 Unicode 规范化形式不同的名称、目录别名，以及通过硬链接指向输入的
输出文件。`--force` 也不允许替换输入图片。

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

`preset link` 和 `preset overlay link` 的本地路径（包括解析符号链接后的目标）必须是有效的
UTF-8。路径无法表示或所需的主目录无法按 UTF-8 展开时，命令会失败，不会链接到拼写不同的
目录；`--create` 也不会绕过此检查。

解除链接只移除当前配置中的 overlay 设置，不删除源文件。移除项目覆盖设置后，可能重新继承
全局 overlay；可运行 `shine preset overlay info` 查看之后实际生效的 overlay。

例如 `app/starship/starship.toml` 会覆盖基础来源中的同路径文件，其它预设继续沿用基础来源。

### 使用 Git 镜像 Overlay

要让多台设备使用同一个只读 overlay 仓库，可由 Shine 管理其本地镜像：

Git 托管 overlay 属于全局配置。请在不受任何 `shine.config.toml` 管理的目录中运行以下命令。
在 Shine 项目中执行 `overlay link --git` 会被拒绝，配置和镜像不会被更改。
如果需要项目专属 overlay，请自行克隆仓库，再在项目中使用 `shine preset overlay link <PATH>`
关联本地目录。

```bash
shine preset overlay link --git https://example.com/team/shine-overlay.git --branch main
shine preset pull
shine preset overlay info
```

关联命令会在 `~/.shine/overlay/` 浅克隆仓库；以后会把该目录镜像到远端分支的最新状态。
`overlay info` 报告托管目录是否存在。
此目录是缓存，任何本地修改都会在下次拉取时丢失。请在仓库上游修改并推送，再在设备上运行 `shine preset pull`、`shine update --pull` 或 `shine upgrade --pull` 同步。

使用不同的 Git URL 重新链接时，现有镜像会切换到该仓库。未指定 `--branch` 时，每次同步都跟随所配置远端的默认分支。拉取失败会保留之前的 checkout。

托管 overlay 必须是该目录内的独立 Git checkout。Shine 会拒绝普通目录、链接的 checkout 和链接的
Git 元数据，并保留其内容；遇到意外占用的目录时，请先将其移到别处，再重试。fetch 成功后，同步会
丢弃托管镜像中的已跟踪修改、未跟踪和被忽略的文件，以及嵌套仓库，即使远端 commit 没有变化。
需要保留本地定制时，请使用手动关联的 overlay。
Git overlay 的 checkout 路径必须能表示为 UTF-8。无法表示的路径会在克隆前被拒绝，
不会改为写入含替换字符的另一个路径。

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

导出内容是内置预设加上当前 overlay 的覆盖和新增文件，不包含仅存在于外部基础目录中的文件；
进度提示会说明是否包含 overlay。Copy 仍只提取原始内置类别。

激活命令中的 `<PATH>` 应替换为实际目录，并加引号作为一个参数传入。

`preset copy` 和 `preset export` 都会拒绝作为输出的链接、特殊文件及链接子目录，即使指定
`--force` 也不例外。请选择普通目录树作为输出位置。未指定 `--force` 时，已有普通文件会保留，
同时运行多个复制命令也不会覆盖其它命令已写入的文件。

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
调用前渲染；渲染失败时该次调用会中止，不会继续运行旧输出。变更 `target`、`runtime`、
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
生效。Live 模式会在下一次命令调用时读取当前文件，同时状态仍会提示刷新已安装命令。完全离线的
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

已有文件时只有加上 `--force` 才会覆盖。未指定时会拒绝已有目录项（包括悬空符号链接），
并发创建也不会互相覆盖。类别格式属于预设作者接口，修改后先运行
`shine preset validate . --format json`，再执行对应的隔离安装 `--dry-run`。Shell 安装 dry-run
只预览命令入口，不安装或修改文件。

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

如果两个条目最终指向同一路径，Shine 会在写入前拒绝整个操作。后续 metadata 若移动已受管 source，`shine upgrade` 只会在旧文件未被修改且新目标不存在时迁移；否则保留两端现状并提示用户处理。

## 声明运行时环境输入

Shell Bun 命令与 App 的 generator、hook 和 artifact 使用结构化 `env`：

```toml
env = [
  { source = "API_URL", sensitivity = "plain" },
  { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" },
]
```

`target` 默认为 `source`。只声明名称与敏感性，不填写值或密文。每个入口只接收自身声明的 Shine
输入，不会获得同类别其他入口的输入。同一有效权限范围内，同一来源的敏感性必须一致。
敏感性描述权限契约，本身不会加密或解密变量。App 保留现有配置值读取行为，Shell 保留密文优先读取。
generator 的 `when_env` 仍填写其 `env` 列表中的来源名称，继续作为独立的启用条件。

预设元数据不再接受 `env = ["SOURCE=TARGET"]` 等旧字符串数组。相关命令会提示迁移，其他命令不会
自动迁移文件。请显式预览并应用转换：

```bash
shine preset migrate ./my-presets --dry-run
shine preset migrate ./my-presets
```

迁移读取旧声明中的显式敏感性，保留别名和条件，并在写入前备份元数据。缺少或冲突的敏感性需要人工
修正。Git 管理的 overlay 必须在上游 checkout 中迁移。迁移不会运行脚本、解密值、授予信任或更新
已安装 launcher 及恢复记录。外部快照变化后需重新审阅信任；迁移后通过正常 install/upgrade 更新安装。
命令行 `shine env run --with SOURCE=TARGET` 语法保持不变。

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

支持 `.ts`、`.js`、`.mts` 和 `.mjs`。安装后，Shine 会创建无扩展名的受管入口，用户仍以 `my-tool` 调用它；现有 `.sh` 和 `.ps1` 原生入口保持兼容。

运行这类命令的每台设备都必须已在 `PATH` 中安装 Bun。Shine 不会安装 Bun 或管理
`node_modules`；外部类别可以按上文约定启用由 Bun 管理的锁定依赖。`runtime = "bun"` 也不能与
`needs_source = true` 组合使用。

可选的 `env` 只适用于 Bun 入口。每项声明 `source`、可选 `target`（默认为来源名称）及必填的 `sensitivity = "plain"` 或 `"secret"`；入口启动时会通过 `shine env run --no-workspace --with ...` 注入值，因此优先解密 `SOURCE_SECRET`，不存在时读取明文 `SOURCE`。声明 `env` 后，运行机器的 `PATH` 中还必须有 `shine`。不要在元数据中填写值或密文，只声明键名。

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

Detection 支持 `command`、`path`，以及由 command/path probes 组成的 `any`。包安装只确保软件存在，不升级已有安装。
复杂 item 可使用 `[items.install] kind = "script", path = "install/<item>.sh"`；脚本只处理该
item，并以普通 exit code 返回结果。每个 init item 都必须同时声明 `detect` 与 `install`，不会再回退到平台级 dispatcher。version 1 manifest 会在 detection 或 profile 写入前被拒绝；请参阅 [v2 迁移指南](sys-preset-v2-migration.md)。

Shell integration 必须且只能声明 `path`、`env`、`eval`、`source`、`aliases` 或 `fragment` 之一。
`profile/base.pre.sh` 与 `profile/base.post.sh` 只放操作系统公共内容；复杂的 item 逻辑放入
`profile/<item>.sh`。phase、可选 priority、manifest 顺序和声明顺序共同决定稳定的组合顺序。
命名 `[profiles.*]` 表只选择 bootstrap items，不定义 shell 内容，也不会禁用选择之外的集成。

外部 sys 安装脚本和可执行 profile 内容（`eval`、`source`、fragment 与 base 文件）要求用户审阅
当前生命周期 Plan，或使用已有匹配 grant；项目配置和 Preset 不能自行授权。类别内容或来源层
变化后快照 grant 会失效；显式开发信任只接受其已登记来源中的代码修改。bootstrap 预检因缺少
信任而停止时尚未运行任何安装器。静态 detection、
package metadata、PATH、env 和 aliases 无需 grant。使用
`shine sys list`、`shine sys info <ITEM>` 和
`shine sys bootstrap <ITEM> --dry-run` 完成验证。

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
运行机器已安装 Bun。只有 `env` allowlist 中的 Shine `[env]` 变量会传给 artifact；每项同时声明来源、
可选目标别名与敏感性，无需在 `[permissions].environment` 重复声明。allowlist 中未配置的可选值会被省略；固定 app 路径变量会
另外加入。若希望安装或升级实际改动
文件后自动构建，可另外声明 `post_install`、`post_upgrade` 钩子；外部预设须人工确认生命周期 Plan
或已有匹配 grant。钩子不能递归调用 artifact 生命周期命令；应使用受支持的 artifact 配置和显式操作。
