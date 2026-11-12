---
title: 快速开始
sidebar_position: 3
---

# 快速开始

下面以代理 Shell 命令为例，完成查看、安装、使用和预览卸载的流程。
请先[安装 Shine](./installation.md)，并准备一个可用的 HTTP 代理服务。
这套预设用于配置代理，不会安装或启动代理服务；没有代理服务时，可改从
[应用预设](./guides/app-presets.md)开始。

## 1. 查看可用预设

```bash
shine shell list
shine app list
shine sys list
shine list --available
```

`list --available` 也可追加 `app`、`shell` 或 `sys` 过滤。

## 2. 安装代理命令

安装前确认代理地址。默认使用 `127.0.0.1:6152`；如果不同，先按
[环境变量指南](./guides/environment.md)使用 `shine env set PROXY_HOST <HOST>` 和
`shine env set HTTP_PROXY_PORT <PORT>` 设置实际地址与端口，再安装。

Shine 会显示将要进行的改动和所需权限，默认为不执行。审阅后确认即可安装命令。

```bash
shine shell install proxy
```

打开一个新终端，或重新加载当前 shell 配置：

```bash
source ~/.zshrc
```

Bash 用户请运行 `source ~/.bashrc`。

PowerShell 用户可重新打开终端，确保更新后的 profile 生效。

## 3. 使用并检查

```bash
setproxy http
shine list
shine info shell/proxy
```

`setproxy http` 为当前终端配置 HTTP 代理，供后续命令使用。如果已安装 Yarn，也会修改其持久
代理设置；关闭终端不会清除这部分设置。

取消当前会话的代理，并清除上述 Yarn 代理设置：

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
- 安装[应用配置](./guides/app-presets.md)，例如 Surge 或 Clash Verge Rev；
- 使用[系统初始化](./guides/system-init.md)初始化机器；
- 保存[可重复任务](./guides/tasks-and-serve.md)；或
- 通过 [SSH 环境转发与文件传输](./guides/ssh-transfer.md)延续工作。
