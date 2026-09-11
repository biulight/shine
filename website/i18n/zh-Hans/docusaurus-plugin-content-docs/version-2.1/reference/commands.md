---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页适用于 Shine 2.1.0。使用 `shine <COMMAND> --help` 查看当前安装版本支持的全部选项。完整操作流程
请阅读对应指南；本页只提供便于查找的命令索引。

## 目标名称

在脚本和文档中使用完整目标：

- 应用配置使用 `app/<category>`；
- Shell 预设使用 `shell/<category>` 或 `shell/<category>/<command>`；
- 受管系统项目使用 `sys/<item>`。

当 App 与 Shell 类别名称不会混淆时，也可以省略类型。单独的 Shell 命令名只用于查看信息。

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

## 顶层命令

| 命令 | 用途 |
| --- | --- |
| `shine init [--yes]` | 为当前项目创建 `shine.config.toml` |
| `shine list [--available [KIND]]` | 查看已安装资源或浏览 `app`、`shell`、`sys` 目录 |
| `shine info <TARGET> [--diff] [--verbose]` | 查看可用或已安装目标的详情 |
| `shine install <TARGET>` | 安装或修复 App/Shell 目标 |
| `shine uninstall <TARGET>` | 卸载 App/Shell 目标 |
| `shine update [TARGET]` | 检查受管内容与 Shine 版本，不应用改动 |
| `shine upgrade [TARGET]` | 应用全部或指定的受管更新 |
| `shine shell ...` / `shine app ...` / `shine sys ...` | 使用资源类型专属操作 |
| `shine preset ...` | 创建、校验和管理预设来源 |
| `shine env ...` | 管理变量、工作区环境、命令代理和密钥 |
| `shine ssh ...` / `shine local ...` | 建立 SSH 会话与传输文件 |
| `shine task ...` / `shine run <NAME>` | 保存和运行个人命令 |
| `shine serve ...` | 提供 `~/.shine/http/` 下的本地资源 |
| `shine self ...` | 安装或升级 Shine 程序 |
| `shine state migrate` | 预览或应用受支持的旧状态迁移 |
| `shine trust ...` | 审阅外部预设代码的目标级信任 |
| `shine completions ...` | 生成或安装 Shell 补全 |
| `shine theme sync` | 输出终端主题环境变量 |

所有命令都接受全局选项 `--config-dir <PATH>`。

## Shell 预设

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes]
shine shell recover [--yes]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes]
```

- 使用 `--dry-run` 预览安装或卸载。
- `--replace-managed` 可能覆盖安装后被修改的受管内容，请先运行 `shine info <TARGET> --diff`。
- 如果 Shine 报告操作曾被中断，请运行 `shine shell recover`。发生过变化的文件会保留，可能需要人工处理。

参见 [管理 Shell 预设](../guides/shell-presets.md)。

## 应用预设

```text
shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes]
shine app refresh <CATEGORY> [FILE] [--force] [--yes]
shine app recover [--yes]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes]
shine app artifact apply <APP_ID> [--yes]
shine app artifact remove <APP_ID> [--yes]
```

- `app info` 和 `update` 只有在传入 `--run-generators` 时才会运行生成器。
- `app refresh` 显式刷新生成文件；`--force` 允许替换用户修改过的受管目标。
- `app uninstall --force` 可能删除用户修改过的受管内容，务必先使用 `--dry-run` 预览。
- App 操作中断并阻塞后续变更时，使用 `shine app recover`。

参见 [管理应用配置](../guides/app-presets.md)。

## 状态、更新、信任与补全

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <app/CATEGORY|sys/ITEM>
shine trust grant <app/CATEGORY|sys/ITEM> [--yes]
shine trust list
shine trust revoke <app/CATEGORY|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

`update` 只读检查；`upgrade` 会显示计划并等待确认。只有审阅过同一范围后才应使用 `--yes`。
`--pull` 会先更新符合条件的 Git 预设来源；`--prune-stale` 允许删除预设中已经不存在且未被修改的受管项。

来源缺失、用户修改、外部命令冲突、权限声明缺失或外部代码尚未信任时，Shine 会提示需要处理，
不会静默覆盖。请恢复来源或按终端给出的命令处理。

## 系统预设

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys recover [--yes]
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes]
shine sys profile enable <ITEM> [--dry-run] [--yes]
shine sys profile disable <ITEM> [--dry-run] [--yes]
shine sys apply [ITEM] [--dry-run] [--yes]
shine sys uninstall <ITEM> [--dry-run] [--yes]
```

使用 `bootstrap` 确保选中的软件和 Shell 集成存在；使用 `apply` 与 `uninstall` 管理可撤销的系统配置。
这些操作可能在计划获批后请求管理员权限。`--force-profile` 可能替换冲突的配置文件内容，请先查看
dry run。

参见 [初始化与管理系统](../guides/system-init.md)。

## 预设创作与来源

```text
shine preset new <app|shell|sys> [--force]
shine preset schema [--format <text|json>]
shine preset validate [PATH] [--format <text|json>]
shine preset lint [PATH] [--format <text|json>] [--deny-warnings]
shine preset plan <CATEGORY> --platform <macos|linux|windows> [--format <text|json>]
shine preset test <CATEGORY> [--format <text|json>]
shine preset pack <CATEGORY> --output <FILE> [--force] [--format <text|json>]
shine preset migrate [PATH] [--dry-run] [--yes] [--format <text|json>]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

发布预设前使用 `validate`、`lint`、`plan` 和 `test`。这些命令只检查创作输入，不会安装预设。
`migrate --dry-run` 用于预览旧 metadata 迁移，实际应用前需要审阅并确认。Git 管理的 overlay 是
可丢弃镜像，应在上游 checkout 中编辑，而不是修改镜像。

参见 [自定义预设](../guides/custom-presets.md)与
[将系统预设迁移到 v2](../guides/sys-preset-v2-migration.md)。

## 环境变量与密钥

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age|hybrid>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity init --phone [--recipient-type <tag|phone>] [--label <LABEL>] [--transport <auto|adb|qr>] [--adb-serial <SERIAL>]
shine env secret identity list
```

`env list --reveal`、`env get`、`env secret decrypt`、带 `--include-secrets` 的导出，以及 `env run`
启动的子进程都可能接触明文。只在可信终端运行，也不要把密钥直接写入命令参数或文档。

Broker policy 命令及完整远程流程见
[SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。本地变量、工作区加密、命令包装和 hybrid
后端见 [管理环境变量与密钥](../guides/environment.md)。

## 任务、本地服务与主题

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine serve install [--port <PORT>]
shine serve start [--port <PORT>]
shine serve status
shine serve uninstall
shine serve url <PATH> [--port <PORT>]

shine theme sync [--auto] [--quiet]
```

任务会运行保存的命令，并可使用保存的工作目录。本地服务会发布 `~/.shine/http/` 下的文件，不要在该
目录放置密钥。

参见 [任务与本地服务](../guides/tasks-and-serve.md)和
[同步终端主题](../guides/terminal-theme-sync.md)。

## SSH 与文件传输

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session] <HOST>
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine 的转发与 broker 选项必须写在 SSH 目标之前。直接转发密钥会让远程会话接触明文。文件传输
仅适用于 POSIX 远端；覆盖现有内容前先使用 `--dry-run`。

参见 [SSH 会话、密钥代理与文件传输](../guides/ssh-transfer.md)。

## 安装和升级 Shine

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

`stable` 跟随正式发布，`preview` 跟随会被持续替换的预览构建。参见
[安装与升级](../installation.md)。
