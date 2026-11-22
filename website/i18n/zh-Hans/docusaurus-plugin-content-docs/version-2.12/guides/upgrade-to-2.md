---
title: 从 Shine 1.x 升级
sidebar_position: 1
---

# 从 Shine 1.x 升级

本页说明如何从既有 1.x 安装升级到 Shine 2.10。请先阅读下列操作确认与恢复变化，
以及[升级与迁移](./upgrading.md)中适用的配置与脚本变更。

## 安装 Shine 2.10

先备份自己的配置与预设，再使用现有 Shine 安装最新稳定版：

```bash
shine self upgrade --channel stable
```

如果旧版本无法自升级，请按[安装与升级](../installation.md)中的平台步骤安装。

## 在变更前审阅 Plan

安装、升级、卸载、generator 刷新、artifact 和受管 Sys 操作现在都会在改动前显示 Plan。
交互式确认默认为 **No**。审阅其中的操作、权限和阻止执行的问题 后再确认，或在有人值守的
自动化中使用 `--yes`：

```bash
shine upgrade app/<CATEGORY>
shine upgrade app/<CATEGORY> --yes
```

`--yes` 只跳过确认提示，Plan 展示、权限检查和执行前的最终校验仍会进行。

## 重新建立外部代码信任

旧的 `allow_app_hooks` 和 `allow_sys_code` 设置已停用，不会自动转换成新的授权。
使用外部 App、Shell 或 Sys 预设时，可以在交互确认中只批准本次列出的代码，
也可以先审阅并保存信任授权，供自动化使用：

```bash
shine trust inspect <TARGET>
shine trust grant <TARGET>
shine trust list
```

使用 `--yes` 执行外部代码前必须已有信任授权。代码或来源变化后需要重新审阅。
`--development` 还会信任同一本地来源的后续代码修改，只有确实接受这一范围时才使用；
Shell 的 live 模式必须使用这种授权。详见[自定义预设](./custom-presets.md)。

## Generator 与环境变化

普通只读检查默认不会运行 App 的生成脚本。如需检查生成结果，使用 `shine info` 或
`shine update` 的 `--run-generators` 选项；外部生成脚本必须先获得信任授权。

如果维护自定义 hook 或生成脚本，请将旧的字符串 `env` 列表转换为结构化输入，声明 `source`、
可选的 `target` 和 `sensitivity`。脚本只会收到已声明的配置输入，不会继承未声明的环境值。
转换步骤见[声明运行时环境输入](./custom-presets.md#声明运行时环境输入)。

## Sys profile 与状态迁移

`shine upgrade` 不再隐式改变 Sys profile 的启用状态，请显式管理：

```bash
shine sys status
shine sys profile enable <ITEM>
shine sys profile disable <ITEM>
```

先预览旧状态和加密配置的迁移，再确认执行：

| 操作 | 命令 |
| --- | --- |
| 预览旧状态迁移 | `shine state migrate --dry-run` |
| 审阅预览后应用迁移 | `shine state migrate` |

旧 App、Shell 和 Sys 安装仍可读取，也能直接升级或移除，无需先重新安装。被用户修改或不属于
Shine 的命令入口和用户文件会保留并报告，不会覆盖。

## 恢复中断的操作

受管操作中断后，后续写操作可能会暂停，直到恢复得到审阅。请使用对应资源类型的命令：

```bash
shine app recover
shine shell recover
shine sys recover
```

恢复只处理自中断后未发生变化的文件。目标文件、备份或恢复文件发生变化时，命令会停止，
并保留这些内容供人工检查。

## 更新外部 Preset

首次升级配置前，先审阅当前使用的 1.x 外部预设与覆盖层（overlay）：

```bash
shine preset migrate --dry-run
shine preset migrate
```

审阅同一来源后用于自动化：

```bash
shine preset migrate --yes
```

也可以传入仓库、类别目录或 `shine.toml`。确认前请审阅差异。迁移会备份并更新元数据，不修改
脚本、数据文件、环境值、已安装资源或信任。Git 管理的 overlay 应在上游 checkout 中迁移并验证，
提交、推送后，再在各设备上运行 `shine preset pull`。

如果报告预设不兼容，请先修复报告中的问题，再升级已安装预设。
自定义 Sys v1 预设需要按[系统预设迁移指南](./sys-preset-v2-migration.md)手动转换。

维护外部预设的用户应在转换后验证来源：

```bash
shine preset validate <PATH>
shine preset lint <PATH> --deny-warnings
```

处理所有错误后，再按上文审阅并重新授权外部代码。权限声明本身不代表授权。
更多编写与验证步骤见[自定义预设](./custom-presets.md)。

请在 [Shine issue tracker](https://github.com/biulight/shine/issues) 反馈升级兼容问题，附上
操作计划 和平台信息，但不要包含 密钥或私有文件内容。
