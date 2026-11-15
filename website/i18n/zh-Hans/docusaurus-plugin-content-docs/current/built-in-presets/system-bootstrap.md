---
title: 系统安装方案
sidebar_position: 8
---

# 系统安装方案

这些预设安装选定的开发工具并配置 Shell 集成，跳过已检测到的软件。
按机器用途从[组合清单](./index.md#系统预设)中选择项目或组合。

预览、批准及 Shell 集成管理方法见[初始化与管理系统](../guides/system-init.md)。

## Arch Linux 准备工作 {#arch-linux-preparation}

仅支持官方 x86_64 Arch Linux，目前只提供默认的 `minimal` 预设，
包含 Neovim、fzf、bat、eza 和 zoxide。

安装前，请先查阅 Arch 的更新说明并完成全系统更新（root 用户省略 `sudo`）。
确认更新成功后，再预览并安装：

```bash
sudo pacman -Syu

shine sys bootstrap --preset minimal --dry-run
shine sys bootstrap --preset minimal
```

Shine 不会自动升级系统。Arch 不支持部分升级，请勿单独运行 `pacman -Sy` 后安装软件。

下载失败并出现 HTTP 404 时，请确认系统更新已成功完成；仍有问题时检查所配置的镜像。
其他错误请根据 pacman 输出排查。参阅 [Arch 系统维护说明](https://wiki.archlinux.org/title/System_maintenance)。

## Ubuntu 安装说明 {#ubuntu-installation-notes}

- **Atuin：** 安装后打开新的 Shell，运行 `atuin import auto` 导入已有历史；需要同步历史时，
  再运行 `atuin register` 或 `atuin login`。
- **Yazi：** 从官方 stable APT 源安装软件及预览所需依赖，需要管理员权限，无需预先安装
  Rust 或 Cargo。已有安装会被跳过，不会自动更换安装方式。
- **Docker Engine：** 包含在 `recommended` 和 `all` 中，按
  [官方 Ubuntu APT 安装步骤](https://docs.docker.com/engine/install/ubuntu/#install-using-the-apt-repository)
  移除发行版冲突包、配置 Docker 签名 stable 源，再安装 Engine、CLI、containerd、Buildx 和 Compose，
  需要管理员权限。移除 `docker.io`、`podman-docker`、`containerd` 或 `runc` 等冲突包可能影响已有工作负载
  和依赖这些包的软件，请先检查安装计划；不会删除容器数据。
  只有五个官方包均完整安装且没有冲突包时才跳过，仅存在 `dockerd` 命令不足以判定完成；缺失组件通过 APT 补齐，过程中可能更新同组的已有包。
  安装后会按需启动 Docker 并运行 `hello-world` 验证，需要能够访问 Docker Hub。
  使用 `sudo docker` 运行命令，Shine 不会将用户加入 `docker` 组；发布容器端口前请查阅 Docker 的防火墙说明。
- **pnpm：** 无需预先安装 Node.js、npm 或 Corepack。安装目录为 `~/.local/share/pnpm`，
  Shine 会配置命令路径；安装后打开新的 Shell 即可使用。
- **最小服务器配置：** `minimal` 仅包含[组合清单](./index.md#系统预设)中列出的工具，不含历史同步、提示符、Node.js 工具链或
  Homebrew。安装前运行 `shine sys bootstrap --preset minimal --dry-run` 确认实际步骤。

## 旧版 Ubuntu 配置中找不到 Rust 命令

升级 Shine，运行 `shine sys bootstrap rust`，然后打开新的 Shell。这会修复集成，不会重装
已检测到的 Rust。Bash/Zsh 中若存在 `$HOME/.cargo/env`，也可运行 `. "$HOME/.cargo/env"` 使当前会话生效。
