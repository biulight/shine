---
title: 从 Shine 1.x 升级
sidebar_position: 1
---

# 从 Shine 1.x 升级

Shine 2.0 目前仍是候选版本。稳定更新通道继续停留在 1.8.x，因此试用 RC 必须显式选择，
日常的稳定版更新检查不会切换到 RC。

## 安装精确的 RC 版本

在 macOS 或 Linux 上下载对应安装器并指定精确版本：

```bash
curl -fsSLO https://github.com/biulight/shine/releases/download/v2.0.0-rc.1/install.sh
SHINE_VERSION=2.0.0-rc.1 sh install.sh
```

在 Windows PowerShell 中：

```powershell
irm https://github.com/biulight/shine/releases/download/v2.0.0-rc.1/install.ps1 -OutFile install.ps1
$env:SHINE_VERSION = "2.0.0-rc.1"; .\install.ps1
```

也可以在 Rust 1.88 或更高版本下安装精确的 crate 版本：

```bash
cargo install shine-cli --version 2.0.0-rc.1
```

`shine self upgrade --channel preview` 跟随持续覆盖的 preview 构建，并不是可复现的 RC。
如需回到稳定系列，请显式重新安装 1.8.x。

## 在变更前审阅 Plan

安装、升级、卸载、generator 刷新、artifact 和受管 Sys 操作现在都会显示绑定状态快照的
Plan。交互式确认默认为 **No**。审阅其中的步骤、权限和 blocker 后再确认，或在有人值守的
自动化中使用 `--yes`：

```bash
shine app upgrade <CATEGORY>
shine app upgrade <CATEGORY> --yes
```

`--yes` 只跳过确认提示，不会跳过 Plan 展示、权限检查，也不会跳过 mutation 前基于最新
快照的再次校验。

## 重新建立外部代码信任

1.x 中宽泛的 `allow_app_hooks` 和 `allow_sys_code` 已停用：它们会被忽略，并在下次保存配置
时移除。Shine 不会将其自动转换成 grant。外部 App、Shell 和 Sys 可执行 target 需要按
target 授权，授权绑定 source layer、代码摘要、capability 和声明的精确权限：

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

外部代码或所请求权限发生变化后，旧 grant 会失效，必须重新审阅。

## Generator 与环境变化

只读的 status 和 info 命令默认不再执行 App generator；仅在明确需要运行 generator 代码
时使用 `--run-generators`。生命周期命令只计算所选操作需要的 generator，并在 Plan 中显示
权限。

hook 和 generator 的环境已收窄到声明的输入。每个必需的环境来源还必须列入 target 的
permission declaration；未声明的值不会从父进程继承。Plan 和信任记录永远不会包含 secret
明文。

## Sys profile 与状态迁移

`shine upgrade` 不再隐式改变 Sys profile 的启用状态，请显式管理：

```bash
shine sys profile status
shine sys profile enable
shine sys profile disable
```

先检查旧 runtime 和 environment 状态，再应用迁移：

```bash
shine state migrate --dry-run
shine state migrate
```

旧 App、Shell 和 Sys manifest 仍可读取。只有相关 mutation 成功后，Shine 才会把 manifest
更新到当前 schema。没有 receipt 的既有 1.8 Shell launcher 可直接生成卸载 Plan 并卸载，
无需先重新安装。被用户修改或不属于 Shine 的 launcher 和用户文件会保留并报告，不会覆盖。

## 恢复中断的操作

journaled mutation 中断后，后续写操作会暂停，直到恢复 Plan 得到审阅。请使用对应生命周期
的命令：

```bash
shine app recover
shine shell recover
shine sys recover
```

恢复只会还原或移除指纹仍匹配的资源。destination、backup 或 rollback 文件发生变化时，
恢复会被阻止，并保留这些内容供人工检查。

## 更新外部 Preset

外部 Preset 必须为每个可执行 target 声明 permission schema v1。缺失或无效声明属于
blocker，不会被解释为隐式宽泛授权。作者应在分发前运行静态和 fixture 检查：

```bash
shine preset schema
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
shine preset test <CATEGORY>
shine preset pack <CATEGORY> --output <FILE>
```

请在 [Shine issue tracker](https://github.com/biulight/shine/issues) 反馈 RC 兼容问题，附上
渲染后的 Plan 和平台信息，但不要包含 secret 或私有文件内容。
