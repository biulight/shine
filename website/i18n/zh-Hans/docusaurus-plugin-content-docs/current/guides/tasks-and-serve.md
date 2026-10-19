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

任务保存在当前 Shine 运行时目录的 `tasks.toml` 中；设置 `SHINE_CONFIG_DIR` 或全局 `--config-dir` 时，会使用对应目录下的任务列表。通过 `--cwd` 保存的任务始终从该目录运行；未设置时继续使用调用者的当前目录。

并发保存和删除使用共享进程锁，保留每次完成的更新。等待其他写入进程最多 30 秒，
超时后报错且不修改任务列表。运行时目录中可能保留 `tasks.lock` 文件；不要通过删除它
来解锁仍在运行的写入进程。

Shine 按参数数组保存并直接执行命令，不会自动经过 shell。因此普通参数边界会被保留，退出码也会原样传递给调用方。需要管道、重定向、变量展开或通配符时，把 shell 写进任务本身，例如 `sh -c '...'`。Windows 上没有系统自带的 `sh`，这类写法只适合 Unix 环境。

在 Unix 上，保存命令的回显使用 POSIX shell 引号，包括可能被当作 shell 语法的程序名
（如 `if` 或 `MODE=test`）。引号只用于展示，任务仍按原始参数数组执行。运行提示写入 stderr，
子进程的 stdout 可继续用于管道。

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

每个环境 source 还必须出现在 `[permissions].environment`；未配置的可选 source 会被省略。值按
存储内容传入；secret 分类输入通过 opaque version 绑定到 Plan，但不会自动解密，也不会触发
GPG、age 或 Touch ID 提示。

App generator、script hook 和 artifact（含 teardown）都从本次捕获的类别快照执行，
相对路径引用的辅助文件也来自该快照。捕获后的来源修改不会改变本次执行的代码。
这些目录在执行结束后清理；需要保留的输出应写入 `SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`，
不要修改执行快照。脚本仍以当前进程权限运行，并未被沙箱隔离。
