---
title: 从 Shine 1.x 升级
sidebar_position: 1
---

# 从 Shine 1.x 升级

Shine 2.0 已是当前稳定版。它引入了更严格的生命周期安全与恢复边界；从既有 1.x 安装升级前，
请先阅读下列兼容性变化。

## 安装稳定版 2.0

在 macOS 或 Linux 上安装最新稳定版：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

在 Windows PowerShell 中：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

也可以在 Rust 1.88 或更高版本下从 crates.io 安装：

```bash
cargo install shine-cli
```

已有 Shine 安装可以显式切换到 stable 通道：

```bash
shine self upgrade --channel stable
```

`shine self upgrade --channel preview` 会改为跟随持续覆盖的 preview 构建，而不是稳定版。

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

首次升级配置前，先审阅当前激活的 1.x external source 与 overlay：

```bash
shine preset migrate --dry-run
shine preset migrate
# 审阅同一来源后用于自动化：
shine preset migrate --yes
```

也可以传入 Preset 仓库、类别目录或 `shine.toml`。该命令只改写安全的 `shine.toml` metadata，
逐文件显示 diff，确认默认是 No；写入前会校验候选内容并创建完整的私有备份集。它绝不会改动
payload、脚本、环境值、运行状态或 trust grant。Git 管理的 overlay 是只读缓存：请对上游 checkout
显式执行路径迁移，在上游提交后再 pull。

opaque App/Shell/Sys 代码的权限与 Sys v1 dispatcher 必须人工改写。按照报告中的 target-local
位置补全。文本报告会为可写 manifest 解析实际路径，并给出可直接复制执行的 `preset validate`
和当前平台 `preset plan` 命令。Git 管理的 overlay 仍是只读镜像，因此报告会指向上游 checkout，
不会建议直接编辑镜像路径。

trust enrollment 只适用于外部 App hook/generator/artifact 代码和 Sys bootstrap/profile 代码。
验证通过后，运行报告给出的 `shine trust inspect app/<CATEGORY>` 或
`shine trust inspect sys/<ITEM>`；只有接受所显示的权限范围后才执行 grant。Shell 命令通过
`[files.permissions]` 声明和 security Plan 审阅，不是合法的 `trust inspect/grant` target。

`shine update` 会显示简洁的 **Preset compatibility** 摘要，并继续完成可执行的配置检查和 Shine
release 检查，最后再因 blocker 返回非零。最终错误只给出一次
`shine preset migrate --dry-run` 入口；该命令再按 manifest 分组显示详细修复步骤。
`shine upgrade` 会在任何生命周期 Plan 或 mutation 前执行同一 preflight；使用 `--pull` 时则在
拉取并重新加载后检查，不兼容时不会产生部分升级。

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
