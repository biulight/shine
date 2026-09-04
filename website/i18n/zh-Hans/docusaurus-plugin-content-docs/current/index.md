---
title: Shine
slug: /
sidebar_position: 1
description: 把日常脚本和配置变成装得上、看得清、中断后也能恢复的个人工具。
---

# Shine

把你反复使用的脚本和配置，变成装得上、看得清、中断后也能恢复的个人工具。

你可能已经把这些文件同步到了多台机器，但文件到了，并不等于马上能用：脚本还要加入 `PATH`，
应用配置要放到正确目录，本地参数也不适合混在共享文件里。手工更新时，还很难判断会不会覆盖
自己原来的内容。个性化配置往往还散落在 Shell 文件、应用目录和系统路径中，时间久了很难维护，
也不方便复用或分享。

Shine 用 **Preset（预设）** 把脚本、个性化配置和安装方式集中整理。Preset 文件夹可以统一维护、
复用和分享，Shine 再把每项内容安装到真正需要它的位置。你可以只安装当前需要的内容，先看清 Shine
准备做什么，不需要时再移除。如果文件已经被修改，或 Shine 无法确认它属于自己，Shine 会停下来，
而不是贸然删除。

**让个人自动化的每一步都看得见，也留有退路。**

当前手册适用于稳定版 **Shine 2.0.1**。冻结的 1.8.x 手册可通过版本选择器查看。

[![Shine 2.0 的三大核心价值：个人脚本与配置可重复部署、个人开发资源的统一入口，以及每次变更看得见、中断后也能恢复。](/img/shine-core-values-v2-zh-Hans.webp)](/img/shine-core-values-v2-zh-Hans.webp)

## 2.0 为什么更安心

- **先说明，再动手：** 安装、升级或移除前，Shine 会先列出准备做什么、需要哪些权限；没有确认就
  不会开始。
- **外部代码逐项确认：** 个人 Preset 需要运行代码时，只需审阅当前这一项；代码或权限改变后，
  Shine 会再次询问。
- **中断后也能安心处理：** 操作中途停止时，Shine 会暂停后续变更并引导你恢复，而不是继续盲目写入。

准备从 1.x 升级，或继续使用自己的 Preset？开始前请先阅读
[从 Shine 1.x 升级](./guides/upgrade-to-2.md)。

## Shine 能替你做什么

- **让脚本像普通命令一样使用：** 安装一次，就能从 `PATH` 中按名字调用。
- **把个性化配置集中起来：** 在一个 Preset 文件夹中维护，Shine 再把每个文件安装到应用真正读取的位置。
- **让每台机器保留自己的参数：** 预设只声明需要哪些键，具体值由每台机器在本地提供。
- **加密凭据只在需要时交给命令：** 将 token 等敏感项目值封存为 GPG 或 age 密文，只对选中的子进程按需解密。
- **先看再改：** 安装、升级或移除前，Shine 会先告诉你准备做什么，再等你确认。
- **不乱动你的文件：** 来源文件和无关内容都会保留；遇到被修改或归属不清的文件，Shine 会先停下来。

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

## 只把凭据交给需要它的命令

凭据不必长期留在工作区文件里，也不必导出到整个 Shell 会话。对于每次调用都要读取固定变量
的 CLI，Shine 可以安装透明命令代理，只在这个命令运行时解析加密值：

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

代理会优先解密 `GH_TOKEN_SECRET`，只把结果注入目标子进程，不会导出回父 Shell。偶尔执行一次
敏感操作时，应改用一次性的 `shine env run --with ... -- <command>`，不必长期启用代理。如何
选择见[管理环境变量与密钥](./guides/environment.md)。

这种方式也能降低明文密钥进入文件、日志、补丁或 AI Agent 上下文的机会，但它并不是沙箱：
目标命令及其后代进程仍然可以读取注入值。完整流程和安全边界见
[在 AI Agent 参与开发时保护环境密钥](./guides/agent-secret-safety.md)。

## 不只处理脚本和配置

你还可以把重复命令保存为[任务](./guides/tasks-and-serve.md)，把选定的值带入
[SSH 会话](./guides/ssh-transfer.md)，并准确决定哪些远程工作流可以拿到某个密钥。Shine 也能
初始化 macOS、Ubuntu 或 Windows 的选定部分，但不会接管第三方工具版本；具体边界见
[系统初始化](./guides/system-init.md)。

## 从这里开始

1. [安装 Shine](./installation.md)
2. [完成第一次预设安装](./quick-start.md)
3. 查看[内置预设](./reference/built-in-presets.md)，看看现在有哪些内容可以直接使用
4. 根据目标阅读 [Shell 预设](./guides/shell-presets.md)、[应用预设](./guides/app-presets.md)、[环境变量与密钥](./guides/environment.md)、[系统初始化](./guides/system-init.md)、[终端主题同步](./guides/terminal-theme-sync.md)、[任务与本地服务](./guides/tasks-and-serve.md) 或 [SSH 会话：密钥代理与文件传输](./guides/ssh-transfer.md)

如果已经遇到问题，直接前往[故障排查](./troubleshooting.md)；需要查看全部选项时再打开[命令参考](./reference/commands.md)。
