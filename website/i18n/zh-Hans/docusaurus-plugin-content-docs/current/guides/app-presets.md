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
shine app uninstall starship --purge
```

install、upgrade、uninstall、generator refresh 和 artifact apply/remove 都会在改动前显示 Plan，
确认默认是 No。只有审阅过同一范围后，才应在有人值守的自动化中使用命令级 `--yes`；它不能绕过
权限、信任或安全检查。

默认情况下，Shine 会保留安装后被修改的文件并报告冲突。安全卸载会还原安装时创建的原文件备份。
迁移目标路径时，旧受管文件必须未修改，新路径也必须空闲；任一位置发生变化，Shine 都会保持原状，
留给用户检查。

`app uninstall --force` 会显式允许删除用户修改过的受管内容，执行前务必使用 `--dry-run` 预览。
`--purge` 还会删除该类别已安装的预设文件。upgrade 仅在获批命令包含 `--prune-stale` 时移除已从
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

Shine 不会隐式运行 artifact 命令。每次手动 apply/remove 都会显示自己的 Plan；脚本失败时命令也会
失败。自动化调用只有在审阅该操作后才应添加 `--yes`。预设作者应让生命周期钩子直接调用底层脚本，
不要在钩子中嵌套 `app artifact apply`。
当某个声明 artifact 的类别在安装或升级中确实改动了受管文件时，Shine 会打印显式 apply 命令；没有受管文件
发生变化时则不提示。脚本只会收到 `[artifact].env` 列出且已配置的 source，并且
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

示例沿用上述三类流量，并为 rule-provider 提供三套互斥布局：mihomo `HomeDir` 内的 `type: file`、同设备的 loopback HTTP 服务，或远程 HTTPS 服务。Shine 会通过普通受管 app 文件把三份默认不生效的参考规则安装到 `HomeDir/ruleset/shine-source/`；只有选择 file provider 时才需要在 overlay 中覆盖这些文件。loopback 和远程 HTTP 布局不会引用它们，因此 URL、interval 与 provider 缓存路径保持不变。`shine upgrade` 实际更新该类别后，会自动运行已审批的 post-upgrade 脚本。若订阅绑定内容已经生效，本地参考规则更新会立即刷新全部 provider 并关闭旧连接；若渲染结果改变了绑定文件，脚本只写入文件，不请求 mihomo 尚未加载的 provider。此时请重新选择订阅，再运行 `shine app artifact apply clash-verge`。选择一整套 provider 后，还需同步启用对应策略组与 `prepend-rules`。loopback 或私有服务的 `proxy: DIRECT` 只控制 provider 下载，如服务器只能经代理访问，应删除或调整它。私有域名依赖系统 split DNS 时，还需配置 mihomo 自己的 `dns.nameserver-policy`。

artifact 和 post-upgrade 脚本都使用 Bun，运行机器必须已安装 Bun。自动钩子仅在 `shine upgrade` 确实改动 `clash-verge` 受管文件时运行；首次设置、重选变化后的绑定或刷新失败重试仍使用显式命令。外部脚本型钩子需要当前 target-scoped trust grant。即时刷新还可使用 `[env]` 中的 `CLASH_CONTROLLER_URL` 和 `CLASH_CONTROLLER_TOKEN`；未配置 URL 时只跳过立即刷新，provider 仍按自身 interval 更新。artifact 会刷新最终生效的 `merge.yaml` 中 `rule-providers` 映射声明的全部名称，自定义 provider 名称无需同步修改脚本。该映射缺失、为 null 或为空时跳过刷新；存在但不是映射时报告配置错误。所有已声明 provider 都刷新成功后，artifact 还会关闭当前全部 mihomo 连接，使浏览器和其它应用自动重连并立即按新规则匹配，无需重启应用；正在进行的下载或其它长连接可能会短暂中断。控制器令牌不要写入 overlay 或文档。

`shine app artifact remove clash-verge` 不会清除 Clash Verge Rev 自己保存的订阅绑定；完全移除时还需在应用中手动清空上述四个编辑器。

## 生命周期钩子

预设作者可以声明 `post_install` 和 `post_upgrade` 钩子：前者在安装实际写入文件后运行，后者只在 `shine upgrade` 实际更新该类别至少一个文件后运行；未变化的类别不会触发。

钩子读取的每个环境输入都必须列入钩子的 `env`，并在类别权限声明中声明同名变量；Plan 不会显示
变量值。command hook 缺少必需输入时不能批准；脚本型 hook 的可选输入缺失时不会注入子进程。

每个钩子必须且只能声明一种动作。`command` 运行声明的命令和参数；`script` 运行 native 或 Bun 脚本，
并只接收自身 `env` 声明的值与固定 `SHINE_APP_*` 变量。`runtime` 只能与 `script` 同用；Bun 脚本沿用
artifact 和 generator 的扩展名及锁定依赖规则。

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

```toml
post_upgrade = [{
  script = "refresh.ts",
  runtime = "bun",
  env = ["API_URL", "API_TOKEN"],
}]

[permissions]
schema_version = 1
filesystem = [{ access = ["execute"], base = "preset", path = "refresh.ts" }]
network = [{ scope = "any" }]
commands = ["bun"]
environment = [
  { name = "API_URL", sensitivity = "plain" },
  { name = "API_TOKEN", sensitivity = "secret" },
]
```

外部预设中的钩子和 generator 需要 target-scoped trust：

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

`trust inspect` 只读，不会授予信任。授予信任前，先处理 Plan 显示的所有缺失权限声明。若启用的 overlay
中 `app/<CATEGORY>/shine.toml` 是覆盖内置 metadata 的旧版完整副本，应先删除或迁移该 metadata 文件；
`merge.yaml`、`rules/` 等 overlay payload 文件仍可保留。

钩子默认不显示 stdout。预设将 `show_output` 设为 `true` 后，安装和 refresh 会显示成功输出；`shine upgrade` 仅在 `--verbose` 下显示成功完成信息和输出。钩子失败或权限拦截始终可见，但不会中断其它类别的安装或升级。
