---
title: 管理 Shell 预设
sidebar_position: 1
---

# 管理 Shell 预设

Shell 预设把常用脚本安装为可以直接调用的命令，支持 Bash、Zsh 和 PowerShell。
部分命令需要 Bun 等额外运行环境，请先确认预设的要求。

内置类别、平台限制、当前会话命令和所需环境变量见[内置预设](../reference/built-in-presets.md#shell-预设)。

## 查看与安装

先查看预设，再选择一种安装范围：

| 操作 | 命令 |
| --- | --- |
| 查看 Shell 预设 | `shine shell list` |
| 安装 proxy 类别 | `shine shell install proxy` |
| 只安装这一个命令 | `shine shell install utils/shine-env-export` |
| 安装当前平台可用的全部类别 | `shine shell install` |

安装类别会启用其中适用于当前平台的全部命令；只需要一个命令时，使用 `category/command`。
不要仅传命令名，因为不同类别可能有同名命令。

执行前请审阅计划中的命令、配置改动、备份和所需权限，确认后才会安装，默认为否。
需要详细路径和权限时，添加 `--verbose`。

Shell 命令以你的权限运行，没有沙箱隔离。外部预设与覆盖层中的代码需要审阅和授权：
交互确认可以只批准本次操作；自动化中使用 `--yes` 前须已有信任授权。
该选项只跳过提示，不会跳过安全检查，且不能与 `--dry-run` 同用。
具体授权步骤见[自定义预设](./custom-presets.md#声明权限)。

安装后需打开新终端或重新加载 shell profile。需要补全时运行：

```bash
shine completions install
```

## 修复安装

命令入口或 Shell 配置损坏时，先检查差异，再决定是否用预设内容替换：

```bash
shine info shell/proxy --diff
shine shell install proxy --replace-managed
```

`--replace-managed` 会覆盖受管内容，包括你的本地修改。

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
恢复 Shell 配置时，Shine 会保留无关内容。

如果相关文件在中断后被修改，Shine 会停止恢复并保留这些文件，避免覆盖你的修改。请保留报错信息和相关文件，根据提示排查，不要手动删除恢复记录或回滚文件。

自动化环境可使用 `shine shell recover --yes` 跳过交互确认；安全检查仍会执行。

## 卸载

| 操作 | 命令 |
| --- | --- |
| 预览卸载类别 | `shine shell uninstall proxy --dry-run` |
| 审阅后执行 | `shine shell uninstall proxy` |
| 另一种范围：仅卸载此命令 | `shine shell uninstall utils/shine-env-export` |

卸载前同样需要审阅并确认计划。按命令卸载会保留同类别中其他已安装命令及它们需要的共享文件。
外部预设源目录和 `~/.shine/config.toml` 始终保留；Shell 配置中的其他内容也会保留。

不指定目标的 `shine shell uninstall` 会卸载全部 Shell 预设，并清理剩余的受管启动配置和整个
内部 Shell 缓存，包括你额外放入缓存的文件。指定类别或命令时，额外缓存文件会保留。
请先用 `shine shell uninstall --dry-run` 核对范围。

卸载和恢复不需要先授权被移除的代码。旧脚本若仍使用 `--purge`，请按
[升级与迁移](./upgrading.md#upgrade-to-shine-26)移除该选项。

## 更新外部预设

默认的 snapshot 模式只在你审阅并执行更新后，才让来源修改进入已安装命令。
使用 `shine update shell/<CATEGORY>` 查看变化，再运行 `shine upgrade shell/<CATEGORY>`。
共享文件的变化可能影响同类别其他已安装命令，请检查计划列出的完整范围；未安装命令不会被自动安装。

live 模式会在下次调用时直接使用修改后的来源，需要 `--development` 授权。
只有确实信任该来源的未来代码修改时才使用它；普通快照授权不能用于 live 模式。
如果 `list` 或 `info` 显示 `development trust required`，请先审阅来源并按提示重新授权。
设置方式见[自定义预设](./custom-presets.md)。

## 内置常用命令

| 类别 | 命令 | 用途 |
| --- | --- | --- |
| `image-tools` | `img-compress`、`img-resize`、`img-convert` | 使用 Bun 1.3.14 或更高版本批量处理 JPEG、PNG、WebP |
| `proxy` | `setproxy`、`usetproxy` | 设置或清除当前终端代理；也会修改 Yarn 的持久代理设置 |
| `utils` | `copyfile` | 通过 OSC52 将文件内容复制到本地剪贴板 |
| `utils` | `shine-env-export` | 将 Shine env 值载入当前 shell |
| `utils` | `shine-theme-sync` | 在当前会话应用终端明暗主题设置 |
| `agent` | `ccenv` | 选择 Codex、DeepSeek 或 Qwen provider，并为启动的 Claude Code 设置专用环境变量；需要 Bun |

某些类别按平台提供不同脚本；`shine shell list` 只显示当前平台可用的条目。

`ccenv` 默认通过本机 `http://127.0.0.1:8317` 的 CLIProxyAPI 使用 Codex，也可交互选择 DeepSeek 或 Qwen。相应凭据使用 `CLIPROXYAPI_AUTH_TOKEN`、`DEEPSEEK_API_KEY` 或 `QWEN_API_KEY`；加密值使用同名的 `_SECRET` 后缀，旧版 `_GPG_SECRET` 仍可读取。所选 provider 的变量只传给启动的 Claude 进程，不会修改当前终端。Claude 参数会原样转发；若首个参数与 `ccenv` 的 `--run` 兼容参数冲突，先写 `--`：

```bash
ccenv --print "hello"
ccenv -- --run
```

想用 Bun 编写跨平台命令预设？请参阅[可选运行时的 Shell 入口](./custom-presets.md#可选运行时的-shell-入口)。
