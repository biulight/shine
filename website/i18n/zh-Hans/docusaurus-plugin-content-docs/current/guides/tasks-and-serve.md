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
`--cwd` 支持 `~`。

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

执行前，Shine 会显示操作计划并请求确认。脚本以你的权限运行，没有沙箱隔离；
请先审阅外部代码。非交互调用须添加 `--yes`，并提前完成外部代码的信任授权。
脚本失败会使命令失败，错误详情显示在终端。

部分预设还会声明安装或升级后的自动操作；这些操作也需要审阅和批准。
`shine update` 只检查更新。具体的应用连接步骤、依赖和移除方式见
[应用预设](./app-presets.md)。

如果你要编写构建脚本，请参阅[自定义预设](./custom-presets.md)，了解运行环境、输入声明和输出目录。
