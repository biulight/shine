---
title: 将系统预设迁移到 v2
sidebar_position: 4
---

# 将系统预设迁移到 v2

系统预设现在使用 `version = 2`。version 1 manifest 会在 Shine 执行 detection、安装器、提权或
profile 写入之前被拒绝。现有 `~/.shine/sys-manifest.toml` 执行记录仍可读取；本迁移改变的是预设
编写契约，不是对已记录软件的所有权。

对每个 init item，请用以下两项替换平台级 `init.sh` 或 `init.ps1` dispatcher：

- 只读的 `detect` 声明（`command`、`path` 或 `any`）；以及
- 使用固定 package provider，或 `install/` 下只处理该 item 的脚本的 `install` 声明。

在 manifest 根部设置 `version = 2`。把软件专属的 Shell 内容移入 `[[items.shell]]`，或移入
item 自己的 `profile/<item>.*` fragment。base profile 文件只能包含平台公共内容。移除 status/update
wire protocol 输出与更新检查 dispatcher；第三方软件更新应使用其包管理器或上游工具。

外部安装脚本、base profile 文件、fragment、`eval` 和 `source` 仍受全局
`allow_sys_code = true` 保护；静态 detection、provider metadata、PATH、环境变量和 aliases 不需要
该授权。可从 `shine preset copy sys/<os>` 开始，并通过以下命令验证：

```bash
shine sys list
shine sys info <ITEM>
shine sys bootstrap <ITEM> --dry-run
```
