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

安装、升级、卸载、generator 刷新、artifact 和受管 Sys 操作现在都会在改动前显示 Plan。
交互式确认默认为 **No**。审阅其中的操作、权限和 blocker 后再确认，或在有人值守的
自动化中使用 `--yes`：

```bash
shine upgrade app/<CATEGORY>
shine upgrade app/<CATEGORY> --yes
```

`--yes` 只跳过确认提示，Plan 展示、权限检查和执行前的最终校验仍会进行。

## 重新建立外部代码信任

1.x 中宽泛的 `allow_app_hooks` 和 `allow_sys_code` 已停用：它们会被忽略，并在下次保存配置
时移除。Shine 不会将其自动转换成 grant。外部 App、Shell 和 Sys 可执行 target 需要按
target 单独授权：

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

外部代码或所请求权限发生变化后，旧的快照 grant 会失效，必须重新审阅。Preset 作者可以显式运行
`shine trust grant <TARGET> --development`，让同一本地来源中的后续代码修改继续受信任；来源或权限
变化后仍须重新审阅。

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
shine sys status
shine sys profile enable <ITEM>
shine sys profile disable <ITEM>
```

先检查旧 runtime 和 environment 状态，再应用迁移：

```bash
shine state migrate --dry-run
shine state migrate
```

旧 App、Shell 和 Sys 安装仍可读取，也能直接升级或移除，无需先重新安装。被用户修改或不属于
Shine 的命令入口和用户文件会保留并报告，不会覆盖。

## 恢复中断的操作

受管操作中断后，后续写操作可能会暂停，直到恢复得到审阅。请使用对应资源类型的命令：

```bash
shine app recover
shine shell recover
shine sys recover
```

恢复只处理自中断后未发生变化的文件。destination、backup 或恢复文件发生变化时，命令会停止，
并保留这些内容供人工检查。

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

trust enrollment 适用于外部 App hook/generator/artifact 代码、Sys bootstrap/profile 代码，以及
声明 unrestricted opaque-code 效果的 Shell command。验证通过后，使用报告给出的规范 target，或用
`shine trust inspect preset` 批量查看当前要求；只有接受所显示的权限范围后才执行 grant。已有的
枚举权限 Shell command 仍只经过权限声明与 security Plan，不会额外获得 trust target。

`shine update` 会显示简洁的 **Preset compatibility** 摘要，并继续完成可执行的配置检查和 Shine
release 检查，最后再因 blocker 返回非零。最终错误只给出一次
`shine preset migrate --dry-run` 入口；该命令再按 manifest 分组显示详细修复步骤。
`shine upgrade` 会在任何生命周期 Plan 或 mutation 前执行同一 preflight；使用 `--pull` 时则在
拉取并重新加载后检查，不兼容时不会产生部分升级。

外部 Preset 必须为每个可执行 target 声明受支持的 permission schema v1 或 v2。缺失或无效声明
属于 blocker，不会被解释为隐式宽泛授权。作者应在分发前运行静态和 fixture 检查：

```bash
shine preset schema
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
shine preset test <CATEGORY>
shine preset pack <CATEGORY> --output <FILE>
```

请在 [Shine issue tracker](https://github.com/biulight/shine/issues) 反馈 RC 兼容问题，附上
渲染后的 Plan 和平台信息，但不要包含 secret 或私有文件内容。
