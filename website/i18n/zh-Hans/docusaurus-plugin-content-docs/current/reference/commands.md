---
title: 命令参考
sidebar_position: 1
---

# 命令参考

本页适用于 Shine 1.3.0。任何子命令都可以使用 `--help` 查看当前安装版本的准确参数。

## 1.0 target 规则

日常命令使用 `app/<category>`、`shell/<category>` 和 `sys/<item>` 作为规范 target。名称在 app 与 shell 间唯一时，安装和卸载也接受裸类别名；脚本和文档中建议始终写完整 target，避免以后新增同名类别后产生歧义。

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

从 1.0 起，`reinstall` 已由 `install --replace-managed` 取代；旧的 `clear`、`pull`、`export`、`link`、`overlay` 顶层入口以及 `app build/unbuild`、`sys init`、`env show` 不再保留兼容别名。

## 顶层命令

| 命令 | 作用 |
| --- | --- |
| `shine init [--yes]` | 在当前项目创建 `shine.config.toml` |
| `shine shell <SUBCOMMAND>` | 管理 Shell 命令预设 |
| `shine app <SUBCOMMAND>` | 管理应用配置预设 |
| `shine install <TARGET> [--replace-managed]` | 安装或修复一个 app/shell target |
| `shine uninstall <TARGET> [--force] [--purge] [--dry-run]` | 卸载一个 app/shell target |
| `shine completions <SUBCOMMAND>` | 生成或安装 Shell 补全 |
| `shine list [--available [KIND]]` | 列出已安装资源，或用 `app`、`shell`、`sys` 浏览可用资源目录 |
| `shine info <TARGET> [--diff] [--verbose]` | 查看可用或已安装的 app/shell target，或 `sys/<ITEM>` |
| `shine update [TARGET]` | 检查受管内容和 Shine 稳定版更新 |
| `shine upgrade [TARGET]` | 应用全部或指定 app、shell、受管 sys 更新 |
| `shine preset <SUBCOMMAND>` | 管理预设来源、overlay、导出和 Git 同步 |
| `shine state migrate [--dry-run]` | 迁移并清理旧版 Shine 运行时状态 |
| `shine self <SUBCOMMAND>` | 安装或升级 Shine 程序 |
| `shine serve <SUBCOMMAND>` | 通过本地 HTTP 服务发布 `~/.shine/http/` 下的资源 |
| `shine env <SUBCOMMAND>` | 管理预设变量、workspace 环境、代理与密钥 |
| `shine sys <SUBCOMMAND>` | 管理系统引导与受管系统配置 |
| `shine theme sync` | 解析终端明暗主题并输出 shell `export` 语句 |
| `shine ssh ...` / `shine local ...` | 开启 SSH 会话、按需代理密钥并在 POSIX 远端传输文件 |
| `shine task <SUBCOMMAND>` / `shine run <NAME>` | 保存和运行个人快捷命令 |

所有命令都支持全局 `--config-dir <PATH>`，用于临时选择全局配置和运行时状态目录。

## Shell 与 App

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [CATEGORY] [--replace-managed]
shine shell uninstall [CATEGORY] [--purge] [--dry-run]

shine app list
shine app info <CATEGORY>
shine app install [CATEGORY] [--dry-run] [--replace-managed]
shine app refresh <CATEGORY> [FILE] [--force]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run]
shine app artifact apply <APP_ID>
shine app artifact remove <APP_ID>
```

`--replace-managed` 会覆盖安装后被用户修改的受管内容。先使用 `shine info <TARGET> --diff` 检查差异。`app uninstall --force` 会删除被用户修改过的受管文件，执行前应加 `--dry-run` 预览。

`app refresh` 只处理 manifest 已跟踪的生成式文件；失败时保留上次成功内容。`app artifact apply/remove` 显式运行预设声明的外部集成脚本，Shine 不会把 apply 隐式作为普通安装或升级的一部分。

## 状态、更新与补全

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale]
shine state migrate [--dry-run]
shine completions install
shine completions <bash|zsh|powershell>
```

- `update --refresh-release` 跳过 24 小时版本检查缓存。`update` 默认复用 `shine list` 的
  Homebrew 风格分栏：交互终端横向排列，重定向输出则保持每行一个 target；末尾只提示
  一次 `shine upgrade`。App 文件按类别折叠。`update --diff` 会改用纵向详细行，展开受
  影响的文件并显示可用内容差异。
- 为 `update` 指定 target 后不能同时使用 `--verbose` 或 `--refresh-release`。
- `update/upgrade --pull` 会先同步 Git 管理的来源并重新加载配置。
- `upgrade --prune-stale` 移除预设来源中已不存在的旧受管 app 文件。
- `upgrade` 默认逐项显示实际更新的 app 类别、Shell target 或受管系统项，并按用户可见
  target 各计数一次；app 行会附带变更文件数。`--verbose` 会展开 app 文件和成功 hook 的
  输出，还会显示已是最新或跳过的项目，以及 snapshot、template、Bin Link 等 Shell
  部署细节。失败、冲突、用户修改警告和被拦截的 hook 无需 `--verbose` 也会显示。
- `shell info` 和顶层 `info` 可以检查尚未安装的预设；`list --available` 可按资源类型过滤。

## 系统预设

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys update [ITEM] [--verbose] [--proxy]
shine sys bootstrap [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy]
shine sys apply [ITEM] [--dry-run]
shine sys uninstall <ITEM> [--dry-run]
```

`sys bootstrap` 安装软件和 shell 集成；`sys update` 只检查已记录的引导软件，不执行升级。独立受管系统项可通过 `shine upgrade sys/<ITEM>` 收敛到当前预设状态。

## 预设来源与定制

```text
shine preset new <app|shell> [--force]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset copy` 只把一个完整的内置预设复制到当前目录，适合创建局部 overlay；`preset export` 导出整套内置预设。外部 Shell 预设默认以快照方式运行，来源内容变更需通过 `shine upgrade` 应用；`--live` 只适合预设开发，令源内容在下一次调用时生效。Git 管理来源的安全限制见[自定义预设](../guides/custom-presets.md)。

## 环境变量与密钥

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env broker describe [--workspace <FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy <add|update> --name <NAME> --ssh-target <TARGET> [--project <PROJECT>] --workspace <FILE> [--remote-workspace <REMOTE_FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy diff <NAME> --workspace <FILE> --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy list
shine env broker policy info <NAME>
shine env broker policy remove <NAME>
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity list
```

`--with` 可重复使用，写成 `KEY=ALIAS` 可改变子进程看到的变量名。`--no-workspace` 只使用显式值和已有进程环境，不能与 `--workspace` 或 `--mode` 同时使用。`workspace init` 只接受 `--from-dotenv`，可先用 `--dry-run` 预览生成文件。broker 策略必须用一个或多个 `--release` 选择密钥，或用 `--release-all-declared` 固化当前环境源声明的全部密钥；二者不能组合。Touch ID identity 只适用于 macOS，并依赖 `age-plugin-se`。

创建 broker 策略时，`--project` 用于保存便于识别的项目标签；`--remote-workspace` 要求远端
请求除了匹配 workspace 内容和其它策略字段外，还必须报告这个完全一致的绝对 workspace 路径。

`env proxy install` 在 `~/.shine/bin/` 创建同名 PATH shim，按规则仅向目标子进程注入 `--with` 指定的值；每个值优先读取 `<KEY>_SECRET`，否则读取 `<KEY>`。`disable` 保留 shim 但跳过解密和注入；项目规则需在当前目录或其祖先存在 `shine.config.toml`，并覆盖同名全局规则。`uninstall` 移除 Shine 管理的 shim 和用户级规则。

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

任务按参数数组保存并直接执行，不经过 shell。`--cwd` 将任务固定到指定工作目录；未设置时继续使用调用者的当前目录。`serve install` 当前只支持 macOS 用户服务，`start` 可在前台启动本地服务。

## SSH 会话、密钥代理与文件传输

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [--secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session]] [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine 自己的选项必须写在 SSH 目标之前。远端按需请求密钥时使用 `shine env run --secret-broker`，详见 [SSH 会话：密钥代理与文件传输](../guides/ssh-transfer.md#按需向远端命令提供密钥)。Windows 远端使用 `--remote-shell windows`，该模式仅提供 PowerShell 环境注入，不建立 `shine local` 传输通道，也不支持 secret broker。

## 程序安装与升级

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

`shine --version` 在稳定版显示 `shine 1.3.0 (<commit> <date>)`；preview 构建使用 `1.3.0-preview` 形式的版本标签。
