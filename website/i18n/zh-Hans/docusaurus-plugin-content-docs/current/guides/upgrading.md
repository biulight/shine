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
再将预设用于自动化。此前 2.9 接受的脚本可能因访问未声明而被阻止。遇到这类诊断时，请修正脚本
或声明所需权限，再运行一次 lint。
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
请从脚本中删除该选项；仍含此选项的命令会失败。

## 从 2.3.x 或更早版本升级：替换 HTTP 服务依赖 {#清理旧版本的本地-http-服务}

Shine 2.4.0 移除了旧 HTTP 服务。如果 Surge、Clash Verge 等应用仍引用它，升级前先改为读取
本地文件。脚本中的 `SHINE_APP_HTTP_DIR` 也需替换；应用 artifact 的持久输出可写入
`SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`。

完成引用调整并升级到 2.4.0 或更高版本后，以安装服务的用户运行：

```bash
shine clear serve
```

此命令会停止旧服务并删除其用户服务注册，即使旧版配置无法加载也可运行。
升级前，也可使用旧二进制的 `shine serve uninstall` 移除服务。

升级不会删除旧 `http/` 目录中的文件。请保留仍需使用的文件，并先更新应用中的引用再删除。

## 从 2.3.x 或更早版本升级：环境分层

此版本调整了环境值优先级：内置默认值 < overlay 基线 < 全局 `[env]` < 全局 env 文件 <
项目 `[env]` < 项目 env 文件。旧版 overlay 和 env 文件的位置不同，升级后实际值可能变化，
不会自动迁移为相同结果。完整规则见[配置参考](../reference/configuration.md#env-值覆盖顺序)。

1. 升级前记录重要项目的变量来源，对准备修改的配置做私有备份，不要把秘密放入公开日志。
2. 更新旧的环境写入 `--force` 用法：全局 env 文件使用 `--global --env-file`，项目 env 文件使用
   `--project --env-file`，手动覆盖层使用 `--overlay`。Git 管理的覆盖层应在上游修改，或改用本地覆盖。
3. 升级后，在项目外及每个重要项目内分别运行 `shine env list` 和 `shine env explain KEY`，
   确认实际值的来源。
4. 旧的显式配置仍然优先，即使它等于过去的默认值。只有确认不再需要时，才指定写入层用
   `shine env delete` 删除，再检查继承结果。调整明文键不会取消同名 `KEY_SECRET` 的优先使用。
5. 运行 `shine update` 检查已安装配置，再审阅并执行 `shine upgrade`。修改环境值或升级程序
   不会自动更新已安装的配置。

回退 Shine 版本会恢复旧的优先级，但不能恢复已删除的值或已更新的应用配置。
需要时从私有备份恢复，并检查是否会覆盖后续修改；没有专用的 env 回退命令。
旧版本可能重新写入默认值，但不会恢复已删除的自定义值。
