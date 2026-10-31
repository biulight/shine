---
title: 管理 Shell 预设
sidebar_position: 1
---

# 管理 Shell 预设

Shell 预设把脚本安装到 Shine 的受管目录，并在 `~/.shine/bin/` 创建可直接调用的命令入口。Shine 当前支持 Bash、Zsh 和 PowerShell 的 profile 与命令目录管理；原生命令入口使用 `.sh` 或 `.ps1`，Bun 可作为另一种跨平台命令运行时。

内置类别、平台限制、当前会话命令和所需环境变量见[内置预设](../reference/built-in-presets.md#shell-预设)。

## 查看与安装

```bash
shine shell list
shine shell install proxy
shine shell install utils/shine-env-export # 只安装这一个命令
shine shell install            # 安装当前平台可用的全部类别
```

也可以使用自动识别 shell 或 app 类别的简写：

```bash
shine install proxy
shine install shell/utils/shine-env-export
```

类别 target 会启用当前平台可用的全部命令；只需要其中一个命令时，使用明确的
`category/command` target。修改型命令不接受裸命令名，因为不同类别可能出现同名命令。

正常的内置 Shell 安装会在确认前显示简短摘要：

```text
Install · utils
  Source       built-in · Shine distribution
  Commands     copyfile, shine-env-export, shine-theme-sync
  Changes      install 3 commands; update shell integration and ~/.bashrc
  Backup       ~/.bashrc

Apply? [y/N]
```

命令、变更的 shell 配置路径和恢复备份路径以当前 Plan 为准。使用 `--verbose` 查看缓存与状态文件、
完整权限、作者说明和计划身份。涉及外部代码、安装期间执行代码、额外权限、冲突或未知诊断时，
仍显示展开的审阅信息。

安装后需打开新终端或重新加载 shell profile。需要补全时运行：

```bash
shine completions install
```

## 修复安装

当受管脚本、命令入口或 PATH 片段需要按当前预设重建时：

```bash
shine shell install proxy --replace-managed
shine install shell/proxy --replace-managed
```

`--replace-managed` 会覆盖 Shine 管理的对应内容。先用 `shine info shell/proxy --diff` 检查状态，避免把有意的本地修改当作损坏处理。

命令与已有文件冲突时，请先检查提示中的路径，再决定是否删除或替换。
切换预设来源或更新 Shine 后，可运行 `shine upgrade shell/<CATEGORY>` 更新已安装命令。
外部代码仍需审阅和授权。

## 安装、升级或卸载中断后怎么办 {#恢复中断的-shell-事务}

若另一项 Shell 操作仍在运行，请等待它结束。如果 Shine 提示已审阅的变更发生变化，
请重新审阅 Plan 后再重试。

如果 Shell 预设操作意外中断，Shine 可能会暂停后续修改操作，并提示先执行恢复：

```bash
shine shell recover
```

查看恢复计划，确认后执行。Shine 会根据中断位置撤销未完成的变更，或保留已完成的操作并清理残留文件。
恢复 Shell profile 中的托管区块时，Shine 会保留无关内容以及 Unix 上当前的文件权限。

如果相关文件在中断后被修改，Shine 会停止恢复并保留这些文件，避免覆盖你的修改。请保留报错信息和相关文件，根据提示排查，不要手动删除恢复记录或回滚文件。

自动化环境可使用 `shine shell recover --yes` 跳过交互确认；安全检查仍会执行。

## 卸载

```bash
shine shell uninstall proxy --dry-run
shine shell uninstall proxy
shine shell uninstall utils/shine-env-export
```

uninstall 和 `shine upgrade` 也会显示各自的计划改动。
确认默认是 No；
自动化必须传入 `--yes`，但 Plan 与安全检查仍会执行。`--dry-run` 是独立预览，不能与 `--yes`
组合。

Shell 命令以你的权限运行，并未被沙箱隔离。External 和 overlay 命令无论如何声明权限，
都需要确认当前 Plan 或具有匹配的信任授权。操作被阻止时，请按提示运行 `shine trust inspect`
和 `shine trust grant` 审阅。只有也信任该来源的后续修改时，才使用 `--development`。
详见[自定义 Preset 能力说明](./custom-presets.md#声明权限)。

Snapshot trust 与默认 snapshot 部署配合使用：Shine 把已审阅类别内容复制到受管目录，之后的来源
修改只有经过显式审阅更新才进入已安装命令。`external_shell_mode = "live"` 必须使用 development
trust，因为以后调用或 source 会读取可变的受信任来源。Snapshot trust 不能用于 live 部署。

更新共享文件可能影响同类别中的其他已安装命令。Plan 会列出所有受影响的命令供审阅；每个受影响 external command 都必须由本次人工确认或匹配 grant 授权。
未安装兄弟不需要 grant，也不会被安装。升级 Shine 后会保留已有 live launcher，但 `list`/`info`
会显示 `development trust required`，直到重新审阅。卸载和恢复无需先信任即将移除的代码。

使用 `shine list` 查看已安装资源；`shine shell list` 展示可用 Shell 预设。
按命令卸载会保留同类别下其他已安装命令；只要兄弟命令仍需要，共享 preset 或 snapshot 文件
就可能继续保留。
卸载最后一个类别后，可以用 `shine shell uninstall` 清理剩余的受管 profile 和启动集成。
Shell 配置中的其他内容会保留；确认前可用 `shine shell uninstall --dry-run` 预览清理范围。

卸载默认清理空的受管目录。未指定 target 时，它会清理整棵内部 Shell 预设缓存树，
包括存放在其中的额外文件。指定类别或命令 target 时，会保留额外缓存文件，以及其他已安装
兄弟命令仍需要的共享文件。外部预设源目录始终保留；适用的受管启动器
和安装 snapshot 仍会被移除。卸载不会删除 `~/.shine/config.toml`。
请审阅 Plan，并用 `--dry-run` 检查所选范围。

`shine uninstall shell/<category>[/<command>]` 使用相同的目标限定行为。`--purge` 已移除；
旧命令请直接去掉该选项。App 类别卸载会删除整个内部类别缓存（包括额外文件），
与 Shell 的目标限定行为不同，详见 [App 预设](app-presets.md)。

## 内置常用命令

| 类别 | 命令 | 用途 |
| --- | --- | --- |
| `image-tools` | `img-compress`、`img-resize`、`img-convert` | 使用 Bun 1.3.14 或更高版本批量处理 JPEG、PNG、WebP |
| `proxy` | `setproxy`、`usetproxy` | 设置或清除当前终端会话的代理变量 |
| `utils` | `copyfile` | 通过 OSC52 将文件内容复制到本地剪贴板 |
| `utils` | `shine-env-export` | 将 Shine env 值载入当前 shell |
| `utils` | `shine-theme-sync` | 输出当前终端明暗主题的 shell `export` 语句 |
| `agent` | `ccenv` | 选择 Codex、DeepSeek 或 Qwen provider，并在隔离的子进程环境中启动 Claude Code；需要 Bun |

某些类别按平台提供不同脚本；`shine shell list` 只显示当前平台可用的条目。

`ccenv` 默认通过本机 `http://127.0.0.1:8317` 的 CLIProxyAPI 使用 Codex，也可交互选择 DeepSeek 或 Qwen。相应凭据使用 `CLIPROXYAPI_AUTH_TOKEN`、`DEEPSEEK_API_KEY` 或 `QWEN_API_KEY`；加密值使用同名的 `_SECRET` 后缀，旧版 `_GPG_SECRET` 仍可读取。所选 provider 的变量只传给启动的 Claude 进程，不会修改当前终端。Claude 参数会原样转发；若首个参数与 `ccenv` 的 `--run` 兼容参数冲突，先写 `--`：

```bash
ccenv --print "hello"
ccenv -- --run
```

想用 Bun 编写跨平台命令预设？请参阅[可选运行时的 Shell 入口](./custom-presets.md#可选运行时的-shell-入口)。
