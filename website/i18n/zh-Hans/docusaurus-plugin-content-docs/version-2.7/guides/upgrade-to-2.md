---
title: 从 Shine 1.x 升级
sidebar_position: 1
---

# 从 Shine 1.x 升级

本页说明如何从既有 1.x 安装升级到 Shine 2.7。请先阅读下列生命周期安全与恢复变化，
以及 [2.7 迁移步骤](../installation.md#升级到-shine-27)。

交互式生命周期 Plan 的人工确认可仅授权本次列出的外部代码，不保存 trust。自动化与 `--yes`
必须已有 grant。Shell live 始终要求 Development trust；显式 `--run-generators` 检查也须已有 grant。

## 安装 Shine 2.7

在 macOS 或 Linux 上安装最新稳定版：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

在 Windows PowerShell 中：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

也可以在 Rust 1.90 或更高版本下从 crates.io 安装：

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
时移除。Shine 不会将其自动转换成 grant。外部 App、Shell 和 Sys 可执行 target 可在交互式生命周期 Plan 中获得本次授权；自动化需按
target 建立持久授权：

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

有效类别内容发生变化后，旧的快照 grant 会失效，必须重新审阅。Preset 作者可以显式运行
`shine trust grant <TARGET> --development`，让同一本地来源中的后续代码修改继续受信任；来源、target 或 capability
变化后仍须重新授权。

## Generator 与环境变化

只读的 status 和 info 命令默认不再执行 App generator；仅在明确需要运行 generator 代码
时使用 `--run-generators`。生命周期命令只计算所选操作需要的 generator，并在 Plan 中显示
权限。

hook 和 generator 的环境已收窄到各自的结构化 `env` 输入。在同一项中声明 `source`、可选
`target` 和 `sensitivity`；环境权限会从这些输入派生，无需重复声明。未声明的值不会从父进程
继承，Plan 和信任记录也不会包含 secret 明文。旧字符串数组须按
[显式迁移步骤](custom-presets.md#声明运行时环境输入)转换。

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

也可以传入仓库、类别目录或 `shine.toml`。确认前请审阅差异。迁移会备份并更新元数据，不修改
脚本、数据文件、环境值、已安装资源或信任。Git 管理的 overlay 应在上游 checkout 中迁移并验证，
提交、推送后，再在各设备上运行 `shine preset pull`。

Sys v1 dispatcher 必须人工迁移。根据报告中的 target 更新来源后，运行
`shine preset validate <PATH>` 和 `shine preset plan <PATH> --platform <PLATFORM>` 校验。
仅缺少可选能力表无需转换元数据；但受支持的 App 代码仍须为静态识别到的行为提供准确声明，
见 [App 代码检查](custom-presets.md#执行前检查-app-代码)。Git 管理的 overlay 仍为只读镜像，
报告会指向上游 checkout。

trust enrollment 适用于外部 App hook/generator/artifact 代码、Sys bootstrap/profile 代码，以及
每个外部 Shell command，与能力表内容无关。验证后使用规范 target，或用 `shine trust inspect preset`
批量查看当前要求；只有接受所显示范围后才执行 grant。旧 grant 需要重新审阅；live Shell 部署必须使用 development trust。

若 Shine 报告预设不兼容，请运行 `shine preset migrate --dry-run` 查看处理步骤。
解决兼容性错误后再升级已安装的预设。

权限表是可选的作者能力说明。显式提供的表必须使用受支持的 schema v1 或 v2 并通过校验；
缺表或有效声明都不会授予代码信任。作者应在分发前运行静态和 fixture 检查：

```bash
shine preset schema
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
shine preset test <CATEGORY>
shine preset pack <CATEGORY> --output <FILE>
```

请在 [Shine issue tracker](https://github.com/biulight/shine/issues) 反馈升级兼容问题，附上
渲染后的 Plan 和平台信息，但不要包含 secret 或私有文件内容。
