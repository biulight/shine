---
title: 升级与迁移
sidebar_position: 0
slug: /upgrading
---

# 升级与迁移

升级前运行 `shine --version` 确认当前版本，并阅读所有适用于来源版本和所用功能的事项。
跨版本升级时，这些要求累积适用；以下分组不表示需要逐个安装中间版本。
根据下列适用事项准备好配置或脚本变更，再按[安装与升级](../installation.md#升级-shine)更新程序。

从 **1.x** 升级时，还需阅读[从 Shine 1.x 升级](./upgrade-to-2.md)。

本页只记录需要提前准备或人工迁移的事项；新增功能与修复列表见
[版本说明](https://github.com/biulight/shine/releases)。

## 升级到 2.10 时检查外部 App 预设 {#upgrade-to-shine-210}

如果你维护含 Bun 脚本的 App 预设，请先用 Shine 2.10 运行 `shine preset lint ./my-presets`，
再将预设用于自动化。2.10 能识别一些 2.9 会漏报的未声明访问，例如通过先前保存的
`Bun.file` 句柄读取文件。遇到阻塞执行的诊断时，请修正脚本或声明所需权限，再运行一次 lint。
详见[执行前检查 App 代码](./custom-presets.md#执行前检查-app-代码)。

## 从 2.7.x 或更早版本升级：SSH 与降级准备 {#upgrade-to-shine-28}

- **使用 Shine SSH 时，先协调两端升级。** 本机与远端一起升级后再重新连接；只更新一端可能中断现有工作流。
- **启用 SSH 文件授权前，先备份策略并阅读[文件授权迁移](./ssh-transfer.md#file-secret-broker)。**
  已有环境变量策略不会自动授权文件，降级程序也不会撤销策略迁移。
- **已安装的 App 需要兼容版本的 Shine 管理。** 新版本写入 App 状态后，2.7.x 及更早版本无法读取。
  降级前应备份配置，并按[升级与降级](./app-presets.md#upgrade-and-downgrade)使用新版本完成卸载。
  不要删除安装记录来绕过兼容性检查。

## 从 2.6.x 或更早版本升级：Preset 环境输入 {#upgrade-to-shine-27}

从 2.6 或更早版本升级时，请在更新外部 Preset 或自动化前检查以下兼容性变化。
源码构建现在要求 Rust 1.90；使用预编译二进制无需安装 Rust。

**转换旧 Preset 环境输入。** Shell Bun 命令及 App 的 generator、hook、artifact 要求结构化
`env`，包含 `source`、可选 `target` 和 `sensitivity`；旧字符串数组会被拒绝。
使用 Shine 2.7 或更高版本预览并应用转换；迁移命令会自动备份修改的元数据：

| 操作 | 命令 |
| --- | --- |
| 预览指定预设源的迁移 | `shine preset migrate ./my-presets --dry-run` |
| 审阅预览后应用迁移 | `shine preset migrate ./my-presets` |
| 验证迁移后的元数据 | `shine preset validate ./my-presets` |
| 检查预设编写问题 | `shine preset lint ./my-presets` |

迁移需要旧声明中的显式敏感性；缺失或冲突时须人工修正。Git 管理的 overlay 要在上游
checkout 中迁移，再重新 pull。迁移会备份元数据，但不会执行脚本、解密值或更新信任。
正常 install/upgrade 前，请重新审阅变化后的外部快照。
详见[运行时环境输入](./custom-presets.md#声明运行时环境输入)。

## 从 2.5.x 或更早版本升级：卸载脚本 {#upgrade-to-shine-26}

Shine 2.6 从 `uninstall`、`app uninstall` 和 `shell uninstall` 中移除了 `--purge`。
请从脚本中删除该选项；仍含此选项的命令会在加载配置前失败。

## 从 2.3.x 或更早版本升级：替换 HTTP 服务依赖 {#清理旧版本的本地-http-服务}

如果应用仍引用旧版 Shine 的 HTTP 服务，升级前先改为读取本地文件。
脚本中的 `SHINE_APP_HTTP_DIR` 也需替换；应用 artifact 的持久输出可写入
`SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`。升级不会删除旧 `http/` 目录中的文件。
完成引用调整后，按[旧服务清理步骤](./system-init.md#retire-legacy-http-service)移除服务。

## 从 2.3.x 或更早版本升级：环境分层

从 2.3.x 或更早版本升级时，请按[环境分层的升级与回退步骤](./environment.md#env-分层变更的人工升级与回退)操作。
更新使用环境写入 `--force` 的脚本，并在升级后于重要项目中运行
`shine env explain KEY`，确认实际生效的值。
