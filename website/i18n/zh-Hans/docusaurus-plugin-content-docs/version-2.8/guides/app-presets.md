---
title: 管理应用配置
sidebar_position: 2
---

# 管理应用配置

应用预设将配置安装到应用需要的位置。替换已有的非受管文件前，会先创建 `*.shine.bak` 备份。

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

```bash
shine app install starship
shine install app/starship
shine update
shine upgrade
```

`shine update` 比较当前安装结果与预设，只报告状态；`shine upgrade` 将受管 shell 和应用配置更新到当前预设内容。

安装失败时，部分文件可能已安装成功。重试前请检查报告的结果。

重新安装类别时会保留安装后被修改的文件，包括受管 JSON 键。对于审阅过的 Plan 标为保留的文件，
Shine 也会跳过其 generator。若要显式覆盖已修改的受管内容，先使用
`shine app install starship --replace-managed --dry-run` 预览，再审阅 `shine app install starship --replace-managed`。

若另一项 App 操作仍在运行，请等待它结束。Shine 提示相关文件或设置发生变化时，请重新审阅 Plan。

如果只需覆盖一个类别的受管文件：

```bash
shine app install starship --replace-managed
```

### 将旧版 App metadata 迁移到 Shine 2

当前 App metadata 会在 `shine.toml` 根级包含以下字段：

```toml
metadata_schema_version = 2
```

它与 `[permissions].schema_version` 相互独立。若旧 overlay 覆盖了
`app/<category>/shine.toml`，但没有 `metadata_schema_version`，先运行
`shine preset migrate --dry-run` 预览，再在审阅 diff 后运行 `shine preset migrate`。也可以显式传入
仓库、类别或 manifest 路径。

迁移只修改 metadata，保留 payload 自定义内容，并在写入前备份文件。无法安全迁移的 hook、generator
和 artifact 会留给作者手动处理。使用 `shine preset validate` 和 `shine preset plan` 验证结果；不要
仅为绕过不兼容而授予外部代码信任。完整作者流程见
[自定义预设](./custom-presets.md#迁移-1x-来源)。

## 卸载与恢复

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
```

install、upgrade、uninstall、generator refresh 和 artifact apply/remove 都会在改动前显示 Plan，
确认默认是 No。只有审阅过同一范围后，才应在有人值守的自动化中使用命令级 `--yes`；它不能绕过
权限、信任或安全检查。

默认情况下，Shine 会保留安装后被修改的文件并报告冲突。安全卸载会还原安装时创建的原文件备份。
迁移目标路径时，旧受管文件必须未修改，新路径也必须空闲；任一位置发生变化，Shine 都会保持原状，
留给用户检查。

生成文件迁移到其他位置时，批准前请检查旧文件及其备份。备份缺失或变化可能需要手动修复；
`shine app recover` 不支持恢复生成文件的位置迁移。

`app uninstall --force` 允许删除用户修改过的受管内容，务必先用 `--dry-run` 预览。
卸载也会删除所选类别的整个 Shine 缓存，包括存放在其中的额外文件；未指定类别时，会清理全部
App 缓存。外部预设来源会保留。请勿将个人文件存放在这些缓存中，并在批准前审阅删除范围。

`shine uninstall app/<category>` 使用相同的类别限定行为。旧的 `--purge` 选项已移除。
请处理受保护或卸载失败的项目后重试，不要手动删除安装记录或备份。

upgrade 仅在获批命令包含 `--prune-stale` 时移除已从
预设中删除且未被修改的受管条目；用户修改过的条目仍会保留。

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

只有确实需要修改受保护路径时，恢复 Plan 才会列出管理员权限。回滚文件可能包含配置数据，应按敏感
内容处理。不要手动编辑或删除恢复状态；若命令报告冲突，请先备份并检查它指出的路径。

## 配置变换

部分预设会在安装前处理源文件，例如：

- `jsonc-to-json`：移除 JSONC 注释和尾随逗号，再写入标准 JSON。
- `template`：用当前 `[env]` 值替换 `@@VAR_NAME@@`。
- `json-merge` 安装模式：只维护目标 JSON 中声明的顶层键，保留其它用户设置。

`shine update` 比较的是变换后的最终结果，而不是原始预设文件。

## 生成式文件与 Surge URI 订阅

`[[files]]` 的 generator 将预期文件内容写入 UTF-8 标准输出，由 Shine 应用变换并管理安装和
卸载。脚本不得直接写入目标文件。stdout 上限为 8 MiB，stderr 上限为 64 KiB，超限会导致生成失败。

生成器可分为自动和手动两类。普通 `list`、`info` 和 `update` 都不会运行它们；无法在不执行代码的
情况下计算动态预期内容时，info/update 会醒目显示 `generator not evaluated`，不会声称已安装文件
是最新状态。使用 `--run-generators` 可以显式执行所选 generator，检查生成结果的状态或差异，而不安装文件：

```bash
shine app info surge --run-generators
shine info app/surge --run-generators --diff
shine update app/surge --run-generators --diff
shine update --run-generators
```

全局形式会评估所有已安装 App 类别；定向形式只评估选中的 App。由于该参数已经表达明确意图，自动
generator 和 `auto = false` 的手动 generator 都会参与。外部或 overlay generator 仍需要匹配的
scoped trust。单项失败不会阻止其余 generator 继续评估，但命令会在报告不完整结果后返回非零状态。
用户取消、命令累计分析预算耗尽，或无法安全继续的分析器故障会停止剩余工作。

受支持的 App Bun 代码会在执行前与声明核对；确定差异或解析失败会阻止该脚本启动，已有 trust
不能绕过检查。其余独立选中文件仍可继续评估。修复方法、覆盖限制和作者工具见
[App 代码检查](custom-presets.md#执行前检查-app-代码)。评估仍不会安装文件、更新 receipt 或运行 hook、artifact。

refresh 只处理已由 App manifest 记录的生成文件。新增带 generator 的 `[[files]]` 项后，先运行一次
`shine install app/<CATEGORY>`，然后才能刷新该 source。

自动 generator 也可以在获批的安装或升级中运行；`auto = false` 的手动 generator 可在安装、显式
评估或显式 refresh 中运行：

```bash
shine app refresh <CATEGORY>
shine app refresh <CATEGORY> <SOURCE_FILE>
```

指定文件时，`SOURCE_FILE` 是预设 `[[files]].source` 的相对路径。刷新失败会保留上次成功内容；
目标已被用户修改时也会保留，只有确认要覆盖时才添加 `--force`。安装和带
`--replace-managed` 的修复安装会运行已由 `when_env` 启用的生成器，不受 `auto` 设置影响。
refresh 会显示并重新校验安全 Plan；自动化调用必须添加 `--yes`。

外部预设或 overlay 的 generator 可通过交互式生命周期确认获得单次授权。
自动化与 `--run-generators` 检查需要先审阅并运行 `shine trust grant app/<CATEGORY>`。
生成器会收到声明的环境输入和固定 `SHINE_APP_*` 路径变量，并受执行时间、输出大小限制。
请只运行你信任的预设。

权限声明用于帮助审阅预设，不会对代码提供沙箱隔离。声明中不得包含密钥值或凭据。
编写预设时请参阅[能力声明](./custom-presets.md#声明权限)。

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

刷新成功且内容变化后会通过现有 `post_upgrade` 钩子 reload Surge；失败时保留上次成功文件。`local-proxy-groups.conf` 中的 `Subscription` 组通过 `policy-path=subscription-proxies.conf` 读取节点，其它策略组可用 `include-other-group=Subscription` 纳入这些节点。

## 构建辅助资源

部分 app 预设会在 `shine.toml` 的 `[artifact]` 中声明脚本。需要生成或刷新这类资源时，手动运行：

```bash
shine app artifact apply surge
```

Shine 不会隐式运行 artifact 命令。每次手动 apply/remove 都会显示自己的 Plan；脚本失败时命令也会
失败。自动化调用只有在审阅该操作后才应添加 `--yes`。预设作者应让生命周期钩子直接调用底层脚本，
不要在钩子中嵌套 `app artifact apply`。
当某个声明 artifact 的类别在安装或升级中确实改动了受管文件时，Shine 会打印显式 apply 命令；没有受管文件
发生变化时则不提示。脚本只会收到结构化 `[artifact].env` 列出且已配置的 source；此外会加入
`SHINE_CACHE_DIR`、`SHINE_STATE_DIR` 等固定路径变量。完整变量说明见
[任务与应用构建产物](./tasks-and-serve.md)。

需要保留的脚本输出请写入 `SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`；临时源文件副本会在执行后删除。

内置 `surge` app 预设会把 `local-proxies.conf`、`local-proxy-groups.conf`、`local-rules.conf` 和可选的订阅生成文件安装到 Surge Profiles 目录。设置 `[env]` 中的 `SURGE_PROFILE` 后，`shine app artifact apply surge` 使用内置 Bun artifact 幂等修补活动配置文件的 `[Proxy]`、`[Proxy Group]` 与 `[Rule]` `#!include` 行。Overlay 只需覆盖自己的策略文件，无需提供构建脚本。

如果旧版 Shine proxy-group 块的标记缺失或嵌套，应用和撤销修补都会报错，并保留配置文件不变。
请先修复配置文件中的异常标记，再重试。

预设还安装默认注释、不立即生效的 `LAN Network`、`LAN PROXY` 和 `Other Direct` 规则示例。每类规则在 `local-rules.conf` 中提供三种互斥来源：随 Profile 安装的相对 `rules/*.list`、同设备 loopback HTTP 地址，或自行替换域名的远程 HTTPS 地址。每类只启用一种；相对文件通常最简单。`localhost` 始终指运行 Surge 的设备，在 iOS 上不会指向另一台局域网主机。

需要撤销这项修补时运行：

```bash
shine app artifact remove surge
```

`artifact remove` 只运行预设声明的 `teardown` 脚本。卸载带有 teardown 的 app 时，Shine 也会尽力执行清理；清理失败只会警告，仍会继续安全卸载受管文件。

### Clash Verge Rev

内置 `clash-verge` 预设提供一个默认无效果的 `merge.yaml` 示例。要叠加自己的代理、策略组、rule-provider 和前置规则，先将完整内置预设复制到自己的局部 overlay：

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/clash-verge
shine preset overlay link .
```

这会创建 `app/clash-verge/`，其中包含当前 Shine 版本附带的 `merge.yaml`、元数据和构建脚本。编辑其中的 `app/clash-verge/merge.yaml`，填入实际配置；不要直接修改 `~/.shine/clash-verge/`，该目录是 Shine 安装后的受管副本。只打算定制 `merge.yaml` 时，可以删除复制出的其它文件：overlay 按相对路径覆盖，缺失的文件会继续使用内置版本并随 Shine 更新。

确认内容后安装预设：

```bash
shine app install clash-verge
```

首次使用时，在 Clash Verge Rev 当前订阅中依次打开并保存 **Extend Config**、**Edit Rules**、**Edit Proxies**、**Edit Groups** 四个订阅级编辑器，然后运行：

```bash
shine app artifact apply clash-verge
```

Shine 只读取 `profiles.yaml` 定位这些绑定文件，不会修改订阅、创建绑定或写入远端订阅 YAML。构建写入新内容后，在 Clash Verge Rev 中重新选择一次订阅，再运行构建即可请求立即刷新 rule-provider。

示例沿用上述三类流量，并为 rule-provider 提供三套互斥布局：mihomo `HomeDir` 内的 `type: file`、同设备的 loopback HTTP 服务，或远程 HTTPS 服务。Shine 会通过普通受管 app 文件把三份默认不生效的参考规则安装到 `HomeDir/ruleset/shine-source/`；只有选择 file provider 时才需要在 overlay 中覆盖这些文件。loopback 和远程 HTTP 布局不会引用它们，因此 URL、interval 与 provider 缓存路径保持不变。`shine upgrade` 实际更新该类别后，会自动运行已审批的 post-upgrade 脚本。若订阅绑定内容已经生效，本地参考规则更新会立即刷新全部 provider 并关闭旧连接；若渲染结果改变了绑定文件，脚本只写入文件，不请求 mihomo 尚未加载的 provider。此时请重新选择订阅，再运行 `shine app artifact apply clash-verge`。选择一整套 provider 后，还需同步启用对应策略组与 `prepend-rules`。loopback 或私有服务的 `proxy: DIRECT` 只控制 provider 下载，如服务器只能经代理访问，应删除或调整它。私有域名依赖系统 split DNS 时，还需配置 mihomo 自己的 `dns.nameserver-policy`。

artifact 和 post-upgrade 脚本都使用 Bun，运行机器必须已安装 Bun。自动钩子仅在 `shine upgrade` 确实改动 `clash-verge` 受管文件时运行；首次设置、重选变化后的绑定或刷新失败重试仍使用显式命令。外部脚本型钩子需要人工确认生命周期 Plan 或已有匹配 grant。即时刷新还可使用 `[env]` 中的 `CLASH_CONTROLLER_URL` 和 `CLASH_CONTROLLER_TOKEN`；未配置 URL 时只跳过立即刷新，provider 仍按自身 interval 更新。artifact 会刷新最终生效的 `merge.yaml` 中 `rule-providers` 映射声明的全部名称，自定义 provider 名称无需同步修改脚本。该映射缺失、为 null 或为空时跳过刷新；存在但不是映射时报告配置错误。所有已声明 provider 都刷新成功后，artifact 还会关闭当前全部 mihomo 连接，使浏览器和其它应用自动重连并立即按新规则匹配，无需重启应用；正在进行的下载或其它长连接可能会短暂中断。控制器令牌不要写入 overlay 或文档。

`shine app artifact remove clash-verge` 不会清除 Clash Verge Rev 自己保存的订阅绑定；完全移除时还需在应用中手动清空上述四个编辑器。

## 生命周期钩子

预设作者可以声明 `post_install` 和 `post_upgrade` 钩子：前者在安装实际写入文件后运行，后者只在 `shine upgrade` 实际更新该类别至少一个文件后运行；未变化的类别不会触发。

钩子读取的每个环境输入都必须在结构化 `env` 中声明 `source`、可选 `target` 与 `sensitivity`，无需重复声明类别环境权限；Plan 不会显示
变量值。缺少必需环境输入时不能批准；脚本型 hook 的可选输入缺失时不会注入子进程。

每个钩子必须且只能声明一种动作。`command` 运行声明的命令和参数；`script` 运行 native 或 Bun 脚本，
并只接收自身 `env` 声明的值与固定 `SHINE_APP_*` 变量。`runtime` 只能与 `script` 同用；Bun 脚本沿用
artifact 和 generator 的扩展名及锁定依赖规则。

```toml
post_upgrade = [
  { command = "my-reloader", env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }] },
]

[permissions]
schema_version = 1
```

```toml
post_upgrade = [{
  script = "refresh.ts",
  runtime = "bun",
  env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }],
}]

[permissions]
schema_version = 1
network = [{ scope = "any" }]
```

外部钩子和 generator 可通过交互式生命周期确认获得本次授权。需要持久授权时使用 target-scoped trust：

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
shine trust grant app/<CATEGORY> --development
```

`trust inspect` 显示需要授权的内容，不会授予信任。Snapshot trust 会绑定完整有效 App 类别，包括辅助文件和数据文件。
只有准备信任所显示本地来源中的后续代码修改时才使用 `--development`。
授予信任前，先处理 Plan 显示的缺失环境输入或 Administrator 要求。若启用的 overlay
中 `app/<CATEGORY>/shine.toml` 是覆盖内置 metadata 的旧版完整副本，应先删除或迁移该 metadata 文件；
`merge.yaml`、`rules/` 等 overlay payload 文件仍可保留。

钩子默认不显示 stdout。预设将 `show_output` 设为 `true` 后，安装和 refresh 会显示成功输出；`shine upgrade` 仅在 `--verbose` 下显示成功完成信息和输出。钩子失败或权限拦截始终可见，但不会中断其它类别的安装或升级。

## 受管秘密文件 {#managed-secret-files}

App metadata v3 可以安装 workspace 密文，并持续管理解密后的目标文件。
明文不得进入预设仓库；继续使用 workspace v3 的 `file_groups` 映射和加密策略，
解密身份留在执行解密的机器上。

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
不要删除 Shine 的私有秘密文件状态或指纹密钥，它们用于所有权检查和恢复。如果中断导致 Shine 未能记录
已创建文件的身份，自动恢复会停止并保留文件供人工检查，即使其内容看起来没有变化。

关闭自动解密时，可在安装后的 workspace 中执行 `shine env workspace export --format files`，
明确选择模式和文件组。手动导出拒绝已有文件，不创建 App 所有权，并使用自身的私有权限默认值；
启动容器前需显式设置服务的读取权限。以后开启自动安装时，可以接管内容相同的文件。

容器运行时仍需明文。配置安装不会启动或重启容器；替换密码文件后，应显式重建需要更新文件挂载的容器。
卸载必要配置前，应先停止依赖它的服务。

使用这些预设前，先升级 SSH 两端。旧客户端会拒绝 App metadata v3 和新版 App manifest，
不会把密文当成普通密码文件安装。
