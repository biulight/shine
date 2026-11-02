---
title: 快速开始
sidebar_position: 3
---

# 快速开始

下面以代理 shell 命令为例，完成一次查看、安装、使用和检查流程。

## 1. 查看可用预设

```bash
shine shell list        # 查看 Shell 预设
shine app list          # 查看 App 预设
shine sys list          # 查看当前平台的系统项目
shine list --available  # 查看所有类型的可用资源
```

`list --available` 也可追加 `app`、`shell` 或 `sys` 过滤。

## 2. 安装代理命令

```bash
shine shell install proxy
# 等价的规范 target 写法：shine install shell/proxy
```

Shine 通过 `~/.shine/bin/` 提供这些命令，并将该目录加入支持的 shell profile。

写入前，Shine 会显示将要进行的改动和所需权限，并以默认 No 等待明确确认。自动化中只有审阅过
同一操作后才应添加 `--yes`；实际修改前仍会显示并校验 Plan。

打开一个新终端，或重新加载当前 shell 配置：

```bash
source ~/.zshrc
# bash 用户使用：source ~/.bashrc
```

PowerShell 用户可重新打开终端，确保更新后的 profile 生效。

## 3. 使用并检查

```bash
setproxy
shine list              # 查看已安装资源
shine info shell/proxy  # 查看命令、入口和来源
```

脚本中使用完整 target `shell/proxy`，裸名称只在 app 与 shell 类别之间唯一时才可使用。

取消当前终端会话中的代理：

```bash
usetproxy
```

## 4. 安全预览卸载

```bash
shine shell uninstall proxy --dry-run  # 预览卸载类别
```

确认输出后去掉 `--dry-run` 即可执行。Shine 只移除自身管理的脚本、命令入口和相关 profile 片段。

接下来可按实际目标继续：

- 把个人脚本或应用配置变成[自定义预设](./guides/custom-presets.md)；
- 安装[应用配置](./guides/app-presets.md)，包括 Surge 与 Clash Verge Rev 的专用 artifact 工作流；
- 使用边界明确的[系统引导](./guides/system-init.md)初始化机器；
- 保存[可重复任务](./guides/tasks-and-serve.md)；或
- 通过 [SSH 环境转发、文件传输与本地 secret broker](./guides/ssh-transfer.md)延续工作。
