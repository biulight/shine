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

## 恢复中断的 Shell 事务

首次安装时，Shine 会先写入 transaction journal，再创建命令 launcher；只有 command manifest
receipt 持久化后才会清理 journal。如果安装在这个窗口中断，后续修改型 Shell 命令会停止，不会
猜测 launcher 是否归 Shine 所有。install 或 upgrade 更新未修改、已有 receipt 的 launcher 时也会
使用同一 journal：旧资源会先移到同目录 `.shine.rollback`，新 receipt 持久化前不会清理这些
rollback material。内置 category cache 的写入也使用同一套 receipt-coherent journal：缺失 cache
文件与 upgrade 或 `--replace-managed` 将要更新的差异文件会被逐一跟踪，替换前已有文件先移到同目录
`.shine.rollback`；跳过的文件与无关 cache 文件不属于本次事务。外部预设使用 snapshot 模式且选中命令无需 rendered output 时，Shine 也会把
共享 category snapshot 的创建或替换写入 journal；全部选中 command receipt 与独立 commit marker
持久化前，旧 category 树会留在确定性的 rollback 目录。install 或 upgrade 产生的 transformed output
使用独立的文件级事务：已有 rendered 文件会移到同目录 `.shine.rollback`，所有消费该路径的 command
receipt 与独立 marker 持久化前，精确旧文件会一直保留。请审阅并执行专用 recovery Plan：

```bash
shine shell recover
shine shell recover --yes # 非交互使用
```

对于尚无 receipt 的 Unix symlink、Unix Bun/live launcher 或 Windows shim，只有它仍与中断创建
的精确状态一致时，恢复才会移除它；launcher 被修改后会保留并阻塞恢复。如果 receipt 已写入，
launcher 会保持安装状态，恢复只清理 stale journal。更新中断时，只有 replacement 与 rollback
资源仍匹配记录的 target、内容 hash 和 mode，恢复才会还原旧 launcher；新 receipt 已持久化后，
恢复会保留 replacement，仅清理未修改的 rollback material。replacement 或 rollback 路径发生变化
会阻塞恢复并保留现场。对于符合条件的 snapshot 事务，commit marker 前的恢复会还原旧的选中
receipt 与精确旧 category 树；marker 后保留 desired 树，只清理精确 rollback。stage、active tree 或
rollback tree 被修改都会阻塞恢复。内置 cache 事务在 marker 前中断时，恢复会移除精确匹配的
事务新建文件，或还原精确旧文件与旧 receipt；marker 后保留 desired 文件，只清理精确 rollback。
任一 cache destination 或 rollback 被修改都会阻塞整个 cache Action，跳过与无关文件保持不变。
rendered 文件事务在 marker 前中断时，恢复会还原旧 receipt 与精确旧文件，或移除精确匹配的
事务新建文件；marker 后保留 desired 文件，只清理精确 rollback。rendered 或 rollback 文件被修改会
阻塞恢复。卸载选择了 rendered 路径的最后一组 consumer receipt 时，Shine 会使用独立事务先记录并
移动精确旧文件，再删除全部 consumer receipt。正向 marker 前，恢复会重建缺失 receipt 并只还原
精确旧文件；marker 后保持路径缺失，只清理精确 rollback。未选中的 consumer 与无关 rendered 文件
保持不变。执行期 live rendering 使用相同 lifecycle lock，pending journal 存在时拒绝运行，但仍是
invocation-scoped atomic write，而非持久事务。cache 与 snapshot 卸载会把精确文件或目录树及其 receipt
transition 写入 journal；正向 marker 前，恢复只还原未修改的 rollback material，marker 后保留已完成
的移除。Shell profile reconciliation 也会记入事务，但只拥有 `# >>> shine >>>` sentinel block：恢复会
把记录的 block transition 合并到当前 profile，并保留中断后出现的无关编辑。Shine-owned block 本身
发生变化时，恢复会阻塞而不是覆盖它。

uninstall 只会对未修改、已有 receipt 的 launcher 使用这项事务。它会先把每个平台 launcher
resource 移到同目录 rollback material，再删除 receipt，随后另行记录持久化 commit marker。如果
中断发生在 receipt 删除之后、marker 写入之前，恢复会先重建旧 receipt，再还原精确资源。marker
写入后，恢复会保留已完成的卸载，只清理未修改的 rollback material。foreign 或已修改 launcher
会被保留在这套 rollback proof 之外。

## 卸载

```bash
shine shell uninstall proxy --dry-run
shine shell uninstall proxy
shine shell uninstall utils/shine-env-export
shine shell uninstall proxy --purge
```

非 dry-run 的 install、uninstall 以及 `shine upgrade` 会显示绑定快照的生命周期 Plan。确认默认
是 No；自动化必须传入 `--yes`，但有序步骤和权限仍会显示并重新校验。`--dry-run` 是独立预览，
不能与 `--yes` 组合。

按命令卸载会保留同类别下其他已安装命令；只要兄弟命令仍需要，共享 preset 或 snapshot 文件
就可能继续保留。`--purge` 会额外删除空的受管预设目录；未指定 target 时会处理整棵 shell
预设目录。它不会删除 `~/.shine/config.toml`。

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
