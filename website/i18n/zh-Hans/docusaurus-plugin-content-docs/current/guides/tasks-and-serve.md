---
title: 任务与应用构建产物
sidebar_position: 6
---

# 任务与应用构建产物

本页说明如何保存个人快捷命令，以及运行 app 的 artifact 脚本。

## 保存常用命令

把一条完整命令保存为任务：

```bash
shine task save logs -- tail -f /var/log/example.log
shine task save kill-port -- sh -c 'lsof -ti :3000 | xargs kill'
shine task save project-check --cwd ~/src/example -- bun run check
```

之后可以从任意目录运行：

```bash
shine task run logs
shine run logs
shine task list
shine task info kill-port
```

任务保存在当前 Shine 配置目录的 `tasks.toml` 中。可用 `SHINE_CONFIG_DIR` 或 `--config-dir`
选择另一份任务列表。通过 `--cwd` 保存的任务始终从该目录运行；未设置时使用当前目录。
`--cwd` 支持 `~`，但要求路径可表示为 UTF-8。

保存或删除任务时，如果因其他命令正在修改列表而超时，请等待该命令结束后重试。
不要通过删除 `tasks.lock` 强制更新。

任务不会自动调用 shell。需要管道、重定向、变量展开或通配符时，请显式使用
`sh -c '...'` 等 shell 命令。Windows 不自带 `sh`，请选用已安装的 shell。
任务输出可通过管道传给其他命令；任务失败时，会返回所运行程序的退出码。

覆盖或删除任务：

```bash
shine task save logs --force -- journalctl -f
shine task delete logs
```

任务名只能包含字母、数字、点、短横线和下划线，并且必须以字母或数字开头。

## 生成 app 辅助资源

部分 app 预设可以声明构建脚本，用来根据当前配置生成额外文件：

```bash
shine app artifact apply surge
```

`app artifact apply` 是显式操作；`shine app install`、`shine update` 和 `shine upgrade` 不会自动
运行 artifact 脚本。执行前会显示安全 Plan，非交互调用必须添加 `--yes`。脚本失败时命令会失败，
脚本输出会直接显示在终端。

构建脚本只会收到 `[artifact].env` 中已配置的 source，以及一组由 Shine 设置的路径变量：

| 变量 | 作用 |
| --- | --- |
| `SHINE_APP_ID` | 当前 app 预设 ID |
| `SHINE_APP_DIR` | 本次执行的临时 app 快照目录，包含生效的基础文件和 overlay 文件 |
| `SHINE_APP_SOURCE_DIR` | 与 `SHINE_APP_DIR` 相同的生效源文件快照目录 |
| `SHINE_APP_OVERLAY_DIR` | 本次快照中生效的 overlay 文件副本；没有 overlay 时显式设为空值，覆盖继承的同名变量 |
| `SHINE_CONFIG_DIR` | 当前 Shine 运行时目录 |
| `SHINE_CACHE_DIR` | 当前 app 的缓存目录 |
| `SHINE_STATE_DIR` | 当前 app 的状态目录 |

每个结构化 `env` 项同时声明来源、可选目标别名和敏感性；未配置的可选 source 会被省略。值按
存储内容传入；secret 分类输入不会自动解密，也不会触发
GPG、age 或 Touch ID 提示。

Generator、hook 和 artifact 从已审阅预设的临时副本运行。需要保留的文件应写入
`SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`；临时副本会在执行后删除。
脚本以你的权限运行，并未被沙箱隔离。
