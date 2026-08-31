---
title: 管理应用配置
sidebar_position: 2
---

# 管理应用配置

应用预设把配置文件安装到目标应用使用的位置，并通过 `~/.shine/app-manifest.toml` 记录受管文件。安装前遇到已有的非受管文件时，Shine 会先创建 `*.shine.bak` 备份。

它们只管理配置文件，不安装、下载或启动对应应用。所有内置类别的目标路径、平台限制、权限与重启要求见[内置预设](../reference/built-in-presets.md#app-预设)。

## 查看与预览

```bash
shine app list
shine app info starship
shine app install starship --dry-run
```

涉及系统目录的预设可能需要额外权限。先使用 `--dry-run` 确认目标路径和变更范围。

## 安装与更新

```bash
shine app install starship
shine install app/starship
shine update
shine upgrade
```

`shine update` 比较当前安装结果与预设，只报告状态；`shine upgrade` 将受管 shell 和应用配置更新到当前预设内容。

如果只需覆盖一个类别的受管文件：

```bash
shine app install starship --replace-managed
```

## 卸载与恢复

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
shine app uninstall starship --purge
```

install、upgrade、uninstall、generator refresh 和 artifact apply/remove 会在 mutation 前显示
绑定快照的 Plan，确认默认是 No；非交互执行使用命令级 `--yes`。该参数仍会显示并重新校验
Plan，不能绕过缺失权限、被阻塞的 teardown 或外部代码 gate。upgrade 仅在审阅命令包含
`--prune-stale` 时移除 App stale 文件。未修改的静态 Copy 与 JSON stale 条目会复用卸载所用的
receipt-gated journal；用户修改过的 stale 内容仍会保留。

当 metadata 把静态 Copy 文件迁移到新的 effective destination 时，upgrade 会把旧 receipt 与
destination、可选固定 backup、rollback 路径和必须为空的新 destination 纳入同一个 relocation
事务。旧受管文件必须未修改（或者在没有 backup 时已经缺失），新路径也必须空闲；新路径被占用或旧
文件被修改时会保留现状并报告冲突。

默认情况下，安装后被用户修改过的文件会保留并标记为用户修改。若安装时创建过备份，安全卸载会恢复原文件。在受支持、已 journal 的静态 Copy 替换不受管 regular-file destination 前，Shine 要求固定的 `<name>.shine.bak` 路径不存在；已有 backup 会阻塞 Plan，并保留两个文件。`--purge` 还会删除相应预设目录；卸载全部类别时也会删除 manifest。

`app uninstall --force` 会显式授权删除被用户修改过的受管内容。对于符合条件的静态 Copy，审阅的
Plan 会标明该 override；事务会先把修改后的文件暂存到 `<name>.shine.rollback`，直至 receipt
commit，并在同一事务中还原可选的固定 backup。管理员静态 Copy 的创建、原地更新和移除使用同一
journal 与 recovery contract；受保护路径的 write、move、mode 还原与 cleanup 会在管理员权限下
执行。JSON merge 的 install、原地 update、普通 uninstall 和强制 uninstall 也会写入 journal。
其他安装策略仍使用原有 lifecycle 路径。执行破坏性操作前请使用 `--dry-run` 预览。

## 恢复中断的 App 操作

在受支持的 App 文件 mutation 之前，Shine 会先写入 operation journal。如果进程在这之后中断，
App install、upgrade、uninstall、refresh 和 artifact 等 mutation 命令会保持阻塞，避免静默
丢弃恢复状态；只读 status/update 检查不会恢复或删除 journal。使用以下命令审阅并应用独立的
recovery Plan：

```bash
shine app recover
# 仅在已经审阅同一 Plan 的非交互环境中使用：
shine app recover --yes
```

对于原本不存在的 destination，只有 transaction-created 文件仍与 Shine 写入的内容逐字节相同，
恢复才会将其删除。对于 backup-aware creation，只有 backup 仍匹配原始内容，且 destination 缺失或
仍匹配受管内容时，恢复才会还原固定 backup；若 backup move 尚未开始，则保留原始 destination。
原地替换 receipt-owned 静态 Copy 时，Shine 会把上一个受管文件临时移动到同目录的
`<name>.shine.rollback`。在 replacement receipt 持久化前，只有 destination 与 rollback 文件仍匹配
前一个/目标 fingerprint 时才会恢复；receipt 持久化后则保留 destination，只移除未修改的 rollback
material 和 stale journal。普通卸载一个未修改、没有 persistent backup 的静态 Copy 时，也会先把受管
文件移动到该 rollback 路径，直到 receipt 移除持久化完成。精确的旧 receipt 仍存在时，恢复会还原该
文件；只有 receipt 移除和 journal 中对应的 commit 状态都已持久化，才会移除 rollback material，
而且仅在其类型、mode 和内容均未变化时执行。receipt 消失但缺少该 journal 状态时属于歧义状态，
恢复会采用保守 rollback：重建旧 receipt，并还原未修改的文件。
如果该静态 Copy 还带有固定 persistent backup，卸载会把两次移动都纳入 journal：先将受管文件移到
`.shine.rollback`，再把 `.shine.bak` 还原到 destination。receipt commit 前，恢复只接受这两次移动
之前、之间或之后产生的精确三路径状态；必要时先把已经还原的用户文件移回 `.shine.bak`，再恢复受管
文件与旧 receipt。receipt commit 后，恢复保留 destination 中未修改的用户文件，只清理未修改的受管
rollback material。两个文件的 mode 与内容 fingerprint 都必须匹配。
强制移除被用户修改过的静态 Copy 时，恢复会分别绑定旧 receipt hash 与修改后文件的
mode/hash。receipt commit 前会还原该精确的修改后文件，并反转可选的 backup restoration；commit
后则保留已完成的卸载，只移除精确匹配修改后文件的 rollback material。
JSON merge 以声明的顶层 key 作为 ownership 边界。已有的完整 JSON object 会被移动到
`.shine.rollback`，但 recovery 只从中读取并还原这些 key，同时保留中断后修改的其它当前值。
如果 destination 原本不存在，只有当前 object 不含其它 key 时才删除整个文件。uninstall receipt
commit 后，当前 JSON object 已归用户所有；即使用户重新加入曾受管的 key，recovery 也只清理未修改
的 rollback material。
对于 `upgrade --prune-stale`，未修改的静态 Copy 与 JSON 条目使用相同的 removal recovery
contract。如果 receipt 已移除但正向 commit marker 尚未持久化，recovery 会重建旧 receipt，并且
只还原精确匹配的 rollback 状态。destination 已缺失时只清理 receipt；此路径绝不会强制移除用户
修改过的 stale 内容。
静态 Copy relocation 的新 receipt 持久化之前，recovery 只会移除未修改的新文件；必要时会把已经
还原到旧 destination 的用户文件放回固定 backup，再恢复精确的旧受管文件。新 receipt 持久化后，
recovery 会保留两端最终状态，只清理未修改的旧 rollback material。JSON relocation 仍使用现有
lifecycle 路径。
当上述创建、更新、relocation 或移除的 recovery 需要修改管理员路径时，recovery Plan 会包含 administrator
permission，Shine 只在该 Plan 获得批准后请求授权。仅重建 receipt 或清理 journal 的恢复不会请求
管理员权限。
rollback 文件可能包含之前的受管配置，应按敏感内容处理。如果任一受保护
路径在中断后被修改，恢复命令返回非零，并保留这些路径和 journal 等待显式处理。把 regular file
替换为 symlink 或目录也视为修改。不要手动编辑或删除 journal 或 rollback material。

## 配置变换

部分预设会在安装前处理源文件，例如：

- `jsonc-to-json`：移除 JSONC 注释和尾随逗号，再写入标准 JSON。
- `template`：用当前 `[env]` 值替换 `@@VAR_NAME@@`。
- `json-merge` 安装模式：只维护目标 JSON 中声明的顶层键，保留其它用户设置。

`shine update` 比较的是变换后的最终结果，而不是原始预设文件。

## 生成式文件与 Surge URI 订阅

App 预设可以为 `[[files]]` 声明 generator，把命令的 UTF-8 stdout 作为该受管文件的预期内容。生成结果仍经过正常的变换、hash、manifest、用户修改保护和卸载流程，不应由脚本绕过 Shine 直接改写目标文件。

生成器可分为自动和手动两类。普通 `list`、`info` 和 `update` 都不会运行它们；无法在不执行代码的
情况下计算动态预期内容时，info/update 会醒目显示 `generator not evaluated`，不会声称已安装文件
是最新状态。使用 `--run-generators` 可以显式执行所选 generator、在内存中应用 transform，并检查
状态或最终 diff，而不会写入目标文件或 manifest：

```bash
shine app info surge --run-generators
shine info app/surge --run-generators --diff
shine update app/surge --run-generators --diff
shine update --run-generators
```

全局形式会评估所有已安装 App 类别；定向形式只评估选中的 App。由于该参数已经表达明确意图，自动
generator 和 `auto = false` 的手动 generator 都会参与。外部或 overlay generator 仍需要匹配的
scoped trust。单项失败不会阻止其余 generator 继续评估，但命令会在报告不完整结果后返回非零状态。

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

外部预设或 overlay 提供的 generator 属于可执行代码，需要审阅后运行
`shine trust grant app/<CATEGORY>`。Shine 只向它传入预设显式声明的 env 值及固定的
`SHINE_APP_*` 路径变量，并限制执行时间和输出大小；仍应只运行自己审阅和信任的预设。

类别根部的 `[permissions]` 会另外声明 generator、hook、artifact 的 command、network scope 和
环境变量敏感度，供静态校验与后续安全 Plan 使用。该声明不会启用或信任外部代码；其中不得写入
URL token、环境变量值、命令参数或密文。

### Surge URI 订阅

内置 `surge` 预设可把 HTTPS Base64 URI 订阅转换为受管的 `subscription-proxies.conf`。此功能需要 Bun，支持兼容的 `ss://` 和 `vmess://` 记录；VLESS、不支持的 transport、插件、坏记录与重复项会被跳过，并只输出不含凭据的摘要。用户维护的 `local-proxies.conf` 不会被改写。

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
shine app install surge
```

该生成器为手动模式，日常 `shine update` 和 `shine upgrade` 不会访问订阅。需要刷新时，先打开 provider 的访问窗口，再运行：

```bash
shine app refresh surge subscription-proxies.conf
```

刷新成功且内容变化后会通过现有 `post_upgrade` 钩子 reload Surge；失败时保留上次成功文件。`local-proxy-groups.conf` 中的 `Subscription` 组通过 `policy-path=subscription-proxies.conf` 读取节点，其它策略组可用 `include-other-group=Subscription` 纳入这些节点。

## 构建辅助资源

部分 app 预设会在 `shine.toml` 的 `[artifact]` 中声明脚本。需要生成或刷新这类资源时，手动运行：

```bash
shine app artifact apply surge
```

Shine 不会隐式运行 artifact；预设可通过生命周期钩子在安装或升级实际改动文件后调用
`app artifact apply`。手动 apply/remove 会显示并重新校验安全 Plan；自动化调用必须添加
`--yes`，执行失败会让命令直接失败。脚本只会收到 `[artifact].env` 列出且已配置的 source，并且
这些 source 还必须在类别 `[permissions].environment` 中声明；此外会加入 `SHINE_APP_HTTP_DIR`、
`SHINE_CACHE_DIR`、`SHINE_STATE_DIR` 等固定路径变量，适合生成放在
`~/.shine/http/app/<APP_ID>/` 下的本地资源。完整变量说明见[任务与本地服务](./tasks-and-serve.md)。

内置 `surge` app 预设会把 `local-proxies.conf`、`local-proxy-groups.conf`、`local-rules.conf` 和可选的订阅生成文件安装到 Surge Profiles 目录。设置 `[env]` 中的 `SURGE_PROFILE` 后，`shine app artifact apply surge` 使用内置 Bun artifact 幂等修补活动配置文件的 `[Proxy]`、`[Proxy Group]` 与 `[Rule]` `#!include` 行。Overlay 只需覆盖自己的策略文件，无需提供构建脚本。

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

示例沿用上述三类流量，并为 rule-provider 提供三套互斥布局：mihomo `HomeDir` 内的 `type: file`、同设备的 loopback HTTP 服务，或远程 HTTPS 服务。Shine 会通过普通受管 app 文件把三份默认不生效的参考规则安装到 `HomeDir/ruleset/shine-source/`；只有选择 file provider 时才需要在 overlay 中覆盖这些文件。loopback 和远程 HTTP 布局不会引用它们，因此 URL、interval 与 provider 缓存路径保持不变。首次升级加入这些受管文件时，可能执行一次预设已有的即时刷新 hook，但不会改变当前 provider 定义。选择一整套 provider 后，还需同步启用对应策略组与 `prepend-rules`。loopback 或私有服务的 `proxy: DIRECT` 只控制 provider 下载，如服务器只能经代理访问，应删除或调整它。私有域名依赖系统 split DNS 时，还需配置 mihomo 自己的 `dns.nameserver-policy`。

该 artifact 使用 Bun，运行机器必须已安装 Bun。预设的安装和升级钩子会在 `merge.yaml` 或受管本地参考规则发生变化后自动再次调用构建；外部预设需要当前 target-scoped trust grant。即时刷新还可使用 `[env]` 中的 `CLASH_CONTROLLER_URL` 和 `CLASH_CONTROLLER_TOKEN`；未配置 URL 时只跳过立即刷新，provider 仍按自身 interval 更新。artifact 会刷新最终生效的 `merge.yaml` 中 `rule-providers` 映射声明的全部名称，自定义 provider 名称无需同步修改脚本。该映射缺失、为 null 或为空时跳过刷新；存在但不是映射时报告配置错误。所有已声明 provider 都刷新成功后，artifact 还会关闭当前全部 mihomo 连接，使浏览器和其它应用自动重连并立即按新规则匹配，无需重启应用；正在进行的下载或其它长连接可能会短暂中断。控制器令牌不要写入 overlay 或文档。

`shine app artifact remove clash-verge` 不会清除 Clash Verge Rev 自己保存的订阅绑定；完全移除时还需在应用中手动清空上述四个编辑器。

## 生命周期钩子

预设作者可以声明 `post_install` 和 `post_upgrade` 钩子：前者在安装实际写入文件后运行，后者只在 `shine upgrade` 实际更新该类别至少一个文件后运行；未变化的类别不会触发。

钩子读取的每个环境输入都必须列入钩子的 `env`，并在类别权限声明中声明同名变量。Plan 审阅会对
`plain` 值取 hash，并以 opaque revision 绑定 `secret` 值；Plan 不会序列化任何原值。缺少钩子输入
或 secret identity 时不能批准。

```toml
post_upgrade = [
  { command = "my-reloader", env = ["API_URL", "API_TOKEN"] },
]

[permissions]
schema_version = 1
environment = [
  { name = "API_URL", sensitivity = "plain" },
  { name = "API_TOKEN", sensitivity = "secret" },
]
commands = ["my-reloader"]
```

外部预设中的钩子和 generator 需要 target-scoped trust：

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

钩子默认不显示 stdout。预设将 `show_output` 设为 `true` 后，安装和 refresh 会显示成功输出；`shine upgrade` 仅在 `--verbose` 下显示成功完成信息和输出。钩子失败或权限拦截始终可见，但不会中断其它类别的安装或升级。
