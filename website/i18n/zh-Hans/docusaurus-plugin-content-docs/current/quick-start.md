---
title: 快速开始
sidebar_position: 3
---

# 快速开始

下面以代理 shell 命令为例，完成一次查看、安装、使用和检查流程。

## 1. 查看可用预设

```bash
shine shell list
shine app list
shine sys list
shine list --available
```

前三个入口分别列出 shell 命令、应用配置和当前操作系统的初始化项目；`list --available` 使用 1.0 的统一目录展示三类资源，也可追加 `app`、`shell` 或 `sys` 过滤。

## 2. 安装代理命令

```bash
shine shell install proxy
# 等价的规范 target 写法：shine install shell/proxy
```

Shine 会把脚本放到 `~/.shine/presets/shell/`，在 `~/.shine/bin/` 创建命令入口，并将该目录加入支持的 shell profile。

打开一个新终端，或重新加载当前 shell 配置：

```bash
source ~/.zshrc
# bash 用户使用：source ~/.bashrc
```

PowerShell 用户可重新打开终端，确保更新后的 profile 生效。

## 3. 使用并检查

```bash
setproxy
shine list
shine info shell/proxy
```

`shine list` 显示已安装的生命周期单元 `proxy`；`shine info shell/proxy` 再展开它管理的命令、
入口与来源详情。脚本中使用完整 target `shell/proxy`，裸名称只在 app 与 shell 类别之间唯一时才可使用。

取消当前终端会话中的代理：

```bash
usetproxy
```

## 4. 安全预览卸载

```bash
shine shell uninstall proxy --dry-run
```

确认输出后去掉 `--dry-run` 即可执行。Shine 只移除自身管理的脚本、命令入口和相关 profile 片段。

接下来可按实际目标继续：

- 把个人脚本或应用配置变成[自定义预设](./guides/custom-presets.md)；
- 安装[应用配置](./guides/app-presets.md)，包括 Surge 与 Clash Verge Rev 的专用 artifact 工作流；
- 使用边界明确的[系统引导](./guides/system-init.md)初始化机器；
- 保存[可重复任务](./guides/tasks-and-serve.md)；或
- 通过 [SSH 环境转发、文件传输与本地 secret broker](./guides/ssh-transfer.md)延续工作。
