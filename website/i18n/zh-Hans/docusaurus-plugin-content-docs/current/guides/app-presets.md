---
title: 管理应用配置
sidebar_position: 2
---

# 管理应用配置

应用预设将配置安装到应用需要的位置。替换已有的非受管配置文件前，会先创建 `*.shine.bak` 备份。
[秘密文件](#managed-secret-files) 使用更严格的冲突规则，不会创建明文备份。

它们只管理配置文件，不安装、下载或启动对应应用。所有内置类别的目标路径、平台限制、权限与重启要求见[内置预设](../reference/built-in-presets.md#app-预设)。

## 查看与预览

```bash
shine app list
shine app info starship
shine app install starship --dry-run
```

涉及系统目录的预设可能需要额外权限。先使用 `--dry-run` 确认目标路径和变更范围。
预览不会写入应用配置文件。

## 安装与更新

安装、升级、卸载、刷新和 artifact 操作都会先显示计划。请审阅后确认，默认为否。自动化中可在
审阅后使用 `--yes`，但该选项不能绕过权限、信任或安全检查。

安装一次，之后按需检查并应用更新：

```bash
shine app install starship
```

等价写法：`shine install app/starship`

```bash
shine update app/starship
shine upgrade app/starship
```

`shine update` 只报告差异，`shine upgrade` 应用可用的配置更新；这里的命令只针对 Starship。

安装失败时，部分文件可能已安装成功。重试前请检查报告的结果。

重新安装类别时会保留安装后被修改的文件，包括受管 JSON 键。若要用预设内容覆盖自己的修改，
请先预览，再批准替换：

```bash
shine app install starship --replace-managed --dry-run
shine app install starship --replace-managed
```

若另一项 App 操作仍在运行，请等待它结束。Shine 提示相关文件或设置发生变化时，请重新审阅 Plan。

### 将旧版 App metadata 迁移到 Shine 2

如果 Shine 提示外部预设或 overlay 的元数据不兼容，先运行 `shine preset migrate --dry-run`
预览，再审阅并执行 `shine preset migrate`。迁移会备份元数据并保留自定义配置文件。安装前请先
处理报告中需要手动修改的项目；授予代码信任不能解决元数据不兼容。验证与编写步骤见
[迁移 1.x 来源](./custom-presets.md#迁移-1x-来源)。

## 卸载与恢复

| 操作 | 命令 |
| --- | --- |
| 预览卸载 | `shine app uninstall starship --dry-run` |
| 审阅预览后再执行卸载 | `shine app uninstall starship` |

默认情况下，Shine 会保留安装后被修改的文件并报告冲突。安全卸载会还原安装时创建的原文件备份。
迁移目标路径时，旧受管文件必须未修改，新路径也必须空闲；任一位置发生变化，Shine 都会保持原状，
留给用户检查。

生成文件迁移到其他位置时，批准前请检查旧文件及其备份。备份缺失或变化可能需要手动修复；
`shine app recover` 不支持恢复生成文件的位置迁移。

`app uninstall --force` 允许删除用户修改过的受管内容，务必先用 `--dry-run` 预览。
即使预设后来更改了安装策略，卸载仍按安装时记录的策略处理文件。
卸载也会删除所选类别的整个 Shine 缓存，包括存放在其中的额外文件；未指定类别时，会清理全部
App 缓存。外部预设来源会保留。请勿将个人文件存放在这些缓存中，并在批准前审阅删除范围。

`shine uninstall app/<category>` 使用相同的类别限定行为。旧的 `--purge` 选项已移除。
请处理受保护或卸载失败的项目后重试，不要手动删除安装记录或备份。

upgrade 仅在获批命令包含 `--prune-stale` 时移除已从
预设中删除且未被修改的受管条目；用户修改过的条目仍会保留。

## 升级与降级 {#upgrade-and-downgrade}

Shine 2.8 及更高版本保存的 App 安装记录无法由 2.7.x 及更早版本读取。降级前：

1. 使用新版程序预览并完成 App 卸载，先处理受保护文件和待恢复操作，并备份需要保留的配置。
2. 若卸载中断，重新执行同一卸载命令，完成清理。
3. 更换程序后，使用旧版支持的预设重新安装。

安装旧程序、重新封存加密源文件或使用 `--replace-managed` 都不能转换安装记录。
不要删除记录或修改其版本号来绕过兼容性检查。

## 恢复中断的 App 操作

App 操作中断后，Shine 可能会阻塞后续改动，以保护尚未处理的状态。只读命令仍可使用。通过以下命令
审阅并应用恢复操作：

```bash
shine app recover
# 仅在已经审阅同一 Plan 的非交互环境中使用：
shine app recover --yes
```

只要相关文件仍与中断时一致，恢复会完成或回退原操作。中断后被修改的文件绝不会被覆盖；恢复会停止
并保留现场，交给用户检查。JSON 恢复只调整预设拥有的键，不影响其它设置。

恢复可能需要管理员权限。恢复文件可能包含配置数据，应按敏感内容处理。不要手动编辑或删除恢复状态；若命令报告冲突，请先备份并检查它指出的路径。

## 配置变换

部分预设会在安装前处理源文件，例如：

- `jsonc-to-json`：移除 JSONC 注释和尾随逗号，再写入标准 JSON。
- `template`：用当前 `[env]` 值替换 `@@VAR_NAME@@`。
- `json-merge` 安装模式：只维护目标 JSON 中声明的顶层键，保留其它用户设置。

`shine update` 比较的是变换后的最终结果，而不是原始预设文件。

## 生成式文件与 Surge URI 订阅

部分预设会根据订阅 URL 等输入生成配置。普通 `list`、`info` 和 `update` 不运行生成器。
`generator not evaluated` 提示表示 Shine 尚未检查生成内容，并不代表已安装文件是最新状态。

若只想检查生成内容而不安装，请选择以下一种范围。这些命令会执行代码，可能访问网络；
外部预设或 overlay 的生成器需要已有的[信任授权](#生命周期钩子)。

| 操作 | 命令 |
| --- | --- |
| 执行生成器并查看状态 | `shine app info surge --run-generators` |
| 执行生成器并查看差异 | `shine info app/surge --run-generators --diff` |
| 执行生成器并检查此 App 的更新 | `shine update app/surge --run-generators --diff` |
| 执行所有已安装 App 类别的生成器 | `shine update --run-generators` |

评估会运行所选范围内的自动和手动生成器，不会安装文件，也不会运行钩子或 artifact。
失败或未完成的评估会报错并返回非零退出状态；请先解决错误，再将结果视为完整检查。
如果 Shine 因代码与声明不一致而阻止脚本执行，请按
[App 代码检查](./custom-presets.md#执行前检查-app-代码)处理；授予信任不能绕过该检查。

要保存新生成的内容，请刷新已安装的文件：

| 操作 | 命令 |
| --- | --- |
| 刷新类别中所有已安装的生成文件 | `shine app refresh <CATEGORY>` |
| 刷新指定文件 | `shine app refresh <CATEGORY> <SOURCE_FILE>` |

`SOURCE_FILE` 是预设列出的源文件相对路径。新增生成文件后，需先运行
`shine app install <CATEGORY>` 安装，再刷新。刷新会显示计划并请求确认，自动化调用需要
`--yes`。失败时保留上次成功内容；本地修改也会保留，只有明确添加 `--force` 才会覆盖。

自动生成器也可以在安装和升级中运行；手动生成器（`auto = false`）在安装、显式评估或刷新时运行，
不会随日常升级执行。使用 `--replace-managed` 重新安装时也可能运行它们。只有符合预设启用条件的
生成器才会执行。

权限声明不会对代码提供沙箱隔离。批准外部脚本前请先审阅；单次授权和持久信任的操作见
[外部代码授权](#生命周期钩子)。

### Surge URI 订阅

内置 `surge` 预设可把 HTTPS Base64 URI 订阅转换为受管的代理文件。`subscription-proxies.conf` 读取 `SURGE_SUBSCRIPTION_URL`；`wget-cloud-proxies.conf` 读取 `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL`。此功能需要 Bun，支持兼容的 `ss://`、`vmess://` 和 `trojan://` 记录，其中 Trojan 支持 TCP 和 WebSocket transport；VLESS、不支持的 transport 或 security mode、插件、坏记录与重复项会被跳过，并只输出不含凭据的摘要。用户维护的 `local-proxies.conf` 不会被改写。

要定制 Surge 的本地代理、策略组或规则文件，先将完整内置预设复制到自己的局部 overlay：

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
shine preset overlay link .
```

编辑复制出的 `app/surge/local-proxies.conf`、`local-proxy-groups.conf` 或 `local-rules.conf`，再安装预设。只打算定制其中部分文件时，可以删除其余复制出的文件：overlay 按相对路径覆盖，缺失的文件会继续使用内置版本并随 Shine 更新。不要直接修改 Surge Profiles 目录中的受管副本。

先配置 URL 并安装：

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
shine env set W_GET_CLOUD_SURGE_SUBSCRIPTION_URL 'https://provider.example/wget-cloud?...'
shine app install surge
```

该生成器为手动模式，日常 `shine update` 和 `shine upgrade` 不会访问订阅。需要刷新时，先打开 provider 的访问窗口，再运行：

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
```

刷新成功且内容变化后会重新加载 Surge；失败时保留上次成功文件。`local-proxy-groups.conf` 中的 `Subscription` 组通过 `policy-path=subscription-proxies.conf` 读取节点，其它策略组可用 `include-other-group=Subscription` 纳入这些节点。

## 构建辅助资源

部分预设需要额外一步，把已安装的文件接入应用。请按提示显式运行，例如：

```bash
shine app artifact apply surge
```

每次 apply 或 remove 都会显示计划并请求确认；脚本失败时，命令也会报告失败。自动化中使用
`--yes` 前应先审阅该操作。这些命令可能修改应用设置，请按下面对应预设的步骤操作。
自定义脚本见[任务与应用构建产物](./tasks-and-serve.md)。

内置 `surge` app 预设会把 `local-proxies.conf`、`local-proxy-groups.conf`、`local-rules.conf`、`shine-general.sgmodule` 和可选的订阅生成文件安装到 Surge Profiles 目录。设置 `[env]` 中的 `SURGE_PROFILE` 后，`shine app artifact apply surge` 会将这些文件接入指定 Profile，启用 `shine-general` 模块，再重新加载 Surge。Overlay 只需覆盖自己的策略文件，无需提供构建脚本。

要添加本地 `[General]` 设置，在当前 overlay 中创建 `app/surge/shine-general.sgmodule`：

```ini
#!name=shine-general
#!desc=Local General settings managed by Shine.
#!system=mac

[General]
skip-proxy = %APPEND% *.internal
```

请保留文件名、`#!name` 和原生模块安装目录 `~/Library/Application Support/Surge/Profiles/`
不变。内置模块只有注释示例。修改 overlay 源文件，不要修改已安装的副本，然后运行：

```bash
shine app install surge
shine env set SURGE_PROFILE "/absolute/path/to/Profile.conf"
shine app artifact apply surge
```

Apply 和 remove 需要 Bun，以及正在运行且 CLI 支持模块管理的 Surge Mac。Apply 会先检查安装的
模块与本次审核过的预设内容是否一致；不一致时，请核对修改并先安装或升级预设。Install 和 upgrade
只部署文件并 reload Surge，不发送模块启用命令。新模块默认关闭，但 Surge 会记住以前的启用状态，
删除后重新安装也可能直接生效。

模块只改变生效配置，不改写原 Profile 的 `[General]`。启用状态属于当前设备，不局限于
`SURGE_PROFILE` 指定的文件。后续可在同一行追加值、新增属性行，或用 `key = value` 覆盖标量设置。
`%APPEND%` 不会对已有值去重，但重复启用或 reload 不会继续累积副本。模块已启用时，安装后的内容
更新会在 reload 时生效。`skip-proxy` 控制系统代理绕过设置，不等同于 `DIRECT` 路由规则；增强模式
仍可能接管相关流量。

如果旧版 Shine proxy-group 块的标记缺失或嵌套，应用和撤销修补都会报错，并保留配置文件不变。
Remove 此时可能已停用模块。请先修复配置文件中的异常标记，再重试。

预设还安装默认注释、不立即生效的 `LAN Network`、`LAN PROXY` 和 `Other Direct` 规则示例。每类规则在 `local-rules.conf` 中提供三种互斥来源：随 Profile 安装的相对 `rules/*.list`、同设备 loopback HTTP 地址，或自行替换域名的远程 HTTPS 地址。每类只启用一种；相对文件通常最简单。`localhost` 始终指运行 Surge 的设备，在 iOS 上不会指向另一台局域网主机。

需要撤销这项修补时运行：

```bash
shine app artifact remove surge
```

`artifact remove` 只运行预设声明的 `teardown` 脚本。卸载带有 teardown 的 app 时，Shine 也会尽力执行清理；清理失败只会警告，仍会继续安全卸载受管文件。
Surge 的 teardown 会先停用 `shine-general` 并 reload，再撤销 include；`artifact remove` 保留
已安装文件，uninstall 随后才删除受管文件。不要先删除模块文件：删除后效果可能持续到下次 reload，
且不会清除记住的启用状态。若已删除，请恢复模块文件并再次运行 remove，以清除该状态。CLI 操作
失败会使 artifact 命令失败。中断的 apply 可能已经更新 include，中断的 remove 可能已经停用模块；
修复提示的问题后，重复原命令即可继续完成。其他模块不会被修改。

### Clash Verge Rev

内置 `clash-verge` 预设提供一个默认无效果的 `merge.yaml` 示例。要叠加自己的代理、策略组、rule-provider 和前置规则，先将完整内置预设复制到自己的局部 overlay：

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/clash-verge
shine preset overlay link .
```

编辑 `app/clash-verge/merge.yaml`，填入实际配置。不要直接修改 `~/.shine/clash-verge/`，
该目录是安装后的受管副本。只定制 `merge.yaml` 时，可以删除其余复制出的文件，让它们继续使用
内置版本并随 Shine 更新。

确认内容后安装预设：

```bash
shine app install clash-verge
```

首次使用时，在 Clash Verge Rev 当前订阅中依次打开并保存 **Extend Config**、**Edit Rules**、**Edit Proxies**、**Edit Groups** 四个订阅级编辑器，然后运行：

```bash
shine app artifact apply clash-verge
```

Shine 会更新这四个编辑器的配置，不修改远端订阅或创建编辑器绑定。写入新内容后，请在 Clash Verge Rev
中重新选择订阅；若要立即刷新规则提供者，再运行一次 apply 命令。

在 `merge.yaml` 中选择一整套规则提供者配置：mihomo 的 `HomeDir` 内的本地文件、同设备
loopback HTTP，或远程 HTTPS，并启用相应的策略组与 `prepend-rules`。只有本地文件方式使用
`HomeDir/ruleset/shine-source/` 下的参考规则；选择此方式时，通过 overlay 定制它们。
`proxy: DIRECT` 只影响规则下载，服务器需要代理时应调整它。私有域名依赖系统 split DNS 时，
还需配置 mihomo 的 `dns.nameserver-policy`。

运行机器必须已安装 Bun。`shine upgrade` 修改此类别后，会自动运行已批准的脚本更新集成。
若编辑器配置发生变化，请重新选择订阅，再运行 `shine app artifact apply clash-verge`。
首次设置和刷新失败后重试，也使用此命令。

如需立即刷新，在 Shine 的环境设置中配置 `CLASH_CONTROLLER_URL` 和 `CLASH_CONTROLLER_TOKEN`。
未配置 URL 时，规则提供者仍按自身间隔更新。立即刷新会处理 `merge.yaml` 声明的全部规则提供者；
全部刷新成功后，会关闭当前 mihomo 连接，让应用重连并按新规则匹配。这可能短暂中断下载和其他长连接。
不要把控制器令牌写入 overlay 或文档。

`shine app artifact remove clash-verge` 不会清除 Clash Verge Rev 自己保存的订阅绑定；完全移除时还需在应用中手动清空上述四个编辑器。

## 生命周期钩子

部分预设会在安装或升级后执行后续操作，例如重新加载应用。这些钩子只在操作写入文件后运行，
未变化的类别不会触发。确认前请检查所需的环境输入和管理员权限。钩子失败会报告错误，但不会
阻止其他类别的安装或升级。需要查看预设允许显示的成功输出时，为 `shine upgrade` 添加 `--verbose`。

外部钩子、生成器和 artifact 需要代码授权。交互确认可以只授权本次列出的代码，不保存信任。
自动化或显式 `--run-generators` 检查需要先审阅并授予持久信任：

| 操作 | 命令 |
| --- | --- |
| 检查代码与所需能力 | `shine trust inspect app/<CATEGORY>` |
| 信任已审阅的类别 | `shine trust grant app/<CATEGORY>` |

授权覆盖类别中列出的全部代码，包括辅助文件；类别内容变化后可能需要重新审阅。
`--development` 也会信任所显示本地来源中的未来代码修改，只有明确接受这一更大范围时才使用。
缺失输入和元数据不兼容仍需处理，权限声明也不会对代码提供沙箱隔离。

钩子声明和环境输入的编写方法见[App 构建脚本运行时](./custom-presets.md#app-构建脚本运行时)。

## 受管秘密文件 {#managed-secret-files}

应用预设可以安装 workspace 密文，并管理服务所需的解密文件。明文不得进入预设仓库，解密身份
应留在执行解密的机器上。先按[管理环境变量](./environment.md)准备 workspace 文件组和加密策略，
再在 App 预设中声明所需文件组：

```toml
metadata_schema_version = 3

[dest]
linux = "/srv/example-service"

[[files]]
source = "compose.yml"

[[secret_files]]
workspace = "shine.workspace.toml"
file_group = "credentials"
mode = "production"
auto_decrypt = true
unix_uid = 1000
unix_gid = 1000
unix_mode = "0400"
```

`workspace` 相对类别目录解析。选中的密文和 workspace 定义会保持相对布局安装，
不要再将它们重复写入 `[[files]]`。明文目标来自 workspace 映射，不得与其他受管资源重叠。
秘密文件不支持 generator、transform、JSON merge 或自动目标目录迁移。

`auto_decrypt` 默认为 `false`。开启后，安装与升级会在 Plan 获批后解密所需文件，
在解密完成前不修改该类别的目标文件。在启用了 SSH broker 的会话中，本机负责解密及硬件授权；
否则使用安装机器已配置的后端。broker 失败不会回退到其他后端。
参见 [SSH 文件授权](ssh-transfer.md#file-secret-broker)。

Unix 默认使用安装用户的所有权和 `0600`。可选的数字 `unix_uid`、`unix_gid`，以及
`unix_mode`（`0400` 或 `0600`）用于设置服务账号的读取权限。
请使用有权创建目标并设置该所有权的账号执行安装。如果指定另一个非 root 所有者，请预先创建所有父目录，
通过 Unix 权限位为该 UID、选定的 GID 或其他用户提供目录遍历（`x`）权限。Shine 会在解密前检查适用的
所有者、组或其他用户权限位；附加组和仅由 ACL 授予的权限不能满足此项检查。已有目录的权限不会自动修改。
Windows 使用当前用户私有 ACL，拒绝 Unix 权限字段。
安装前应检查权限预览。

已有非受管目标只有在内容与解密结果完全相同时才会被接管。内容不同即冲突，
`--replace-managed` 也不会覆盖这种文件；不会生成明文 `.shine.bak`。
后续本地修改默认保留，只有显式替换才会覆盖。未变化的文件跳过解密。
只读命令和 diff 仅显示状态，不解密、不展示内容。

卸载会移除未修改的受管秘密文件，保留本地修改。关闭自动解密会暂停创建和更新，但保留既有所有权。
删除声明后，需要通过 `upgrade --prune-stale` 移除未修改的旧目标；目标迁移需要先显式处理旧安装。
写入中断会保留受保护的恢复状态，重试前应审阅并执行 `shine app recover`。
不要删除 Shine 的私有秘密文件状态或密钥。无法安全确认文件身份时，恢复会保留文件供人工检查，
即使其内容看起来没有变化。

关闭自动解密时，可在安装后的 workspace 中执行 `shine env workspace export --format files`，
明确选择模式和文件组。手动导出拒绝已有文件，不创建 App 所有权，并使用自身的私有权限默认值；
启动容器前需显式设置服务的读取权限。以后开启自动安装时，可以接管内容相同的文件。

容器运行时仍需明文。配置安装不会启动或重启容器；替换密码文件后，应显式重建需要更新文件挂载的容器。
卸载必要配置前，应先停止依赖它的服务。

使用 `secret_files` 的预设必须声明 `metadata_schema_version = 3`。通过 SSH 使用这些预设前，
请先升级两端。改用旧版 Shine 前，请阅读[升级与降级](#upgrade-and-downgrade)。
