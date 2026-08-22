---
title: Shine
slug: /
sidebar_position: 1
description: 把日常脚本和配置变成随时装得上、更新得了、删得干净的个人工具。
---

# Shine

把你反复使用的脚本和配置，变成随时装得上、更新得了、删得干净的个人工具。

你可能已经把这些文件同步到了多台机器，但文件到了，并不等于马上能用：脚本还要加入 `PATH`，
应用配置要放到正确目录，本地参数也不适合混在共享文件里。手工更新时，还很难判断会不会覆盖
自己原来的内容。个性化配置往往还散落在 Shell 文件、应用目录和系统路径中，时间久了很难维护，
也不方便复用或分享。

Shine 用 **Preset（预设）** 把脚本、个性化配置和安装方式集中整理。Preset 文件夹可以统一维护、
复用和分享，Shine 再把每项内容安装到真正需要它的位置。你可以只安装当前需要的内容，先看变化
再更新，不需要时再安全移除，不会顺手删掉无关文件。

**让个人自动化拥有可审阅的生命周期。**

当前手册适用于 **Shine 1.6.0**。

## Shine 能替你做什么

- **让脚本像普通命令一样使用：** 安装一次，就能从 `PATH` 中按名字调用。
- **把个性化配置集中起来：** 在一个 Preset 文件夹中维护，Shine 再把每个文件安装到应用真正读取的位置。
- **让每台机器保留自己的参数：** 预设只声明需要哪些键，具体值由每台机器在本地提供。
- **更新前先看一眼：** 默认先查看发生了什么变化，再由你决定何时升级应用。
- **不用时只删 Shine 安装的内容：** 来源文件夹和无关文件都会保留。

## 先试一个内置预设

安装图片处理命令、缩放一张图片，再看看 Shine 加入了哪些内容：

```bash
shine list --available
shine install shell/image-tools
img-resize photo.jpg
shine info shell/image-tools
```

你还可以查看 Starship、Git、Vim、Ghostty 等工具已有的配置预设。Surge 与 Clash Verge Rev
有各自的引导流程，详见[应用预设](./guides/app-presets.md)。

## 再把自己的习惯做成预设

预设文件夹可以通过任意文件夹同步工具、压缩包、网络传输、版本管理工作区或手工复制到达机器，
Shine 不限定你怎样分享它。

内置 `image-tools` 预设已经用可复用的压缩、缩放和格式转换命令展示了这种模式。你也可以用同样
方式把批量重命名、表格整理或文档打印封装成自己的命令。每个命令需要的应用或运行时仍要安装在
对应机器上。[自定义预设](./guides/custom-presets.md)介绍了实现机制和完整图片工作流。

## 不只处理脚本和配置

你还可以把重复命令保存为[任务](./guides/tasks-and-serve.md)，把选定的值带入
[SSH 会话](./guides/ssh-transfer.md)，并准确决定哪些远程工作流可以拿到某个密钥。Shine 也能
初始化 macOS、Ubuntu 或 Windows 的选定部分，但不会接管第三方工具版本；具体边界见
[系统初始化](./guides/system-init.md)。

## 从这里开始

1. [安装 Shine](./installation.md)
2. [完成第一次预设安装](./quick-start.md)
3. 查看[内置预设](./reference/built-in-presets.md)，看看现在有哪些内容可以直接使用
4. 根据目标阅读 [Shell 预设](./guides/shell-presets.md)、[应用预设](./guides/app-presets.md)、[系统初始化](./guides/system-init.md)、[终端主题同步](./guides/terminal-theme-sync.md)、[任务与本地服务](./guides/tasks-and-serve.md) 或 [SSH 会话：密钥代理与文件传输](./guides/ssh-transfer.md)

如果已经遇到问题，直接前往[故障排查](./troubleshooting.md)；需要查看全部选项时再打开[命令参考](./reference/commands.md)。
