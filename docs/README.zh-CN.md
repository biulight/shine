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

**完整文档：**[简体中文](https://biulight.github.io/shine/zh-Hans/) ·
[English](https://biulight.github.io/shine/)

## Shine 能替你做什么

- **让脚本像普通命令一样使用：** 安装一次，就能从 `PATH` 中按名字调用。
- **把个性化配置集中起来：** 在一个 Preset 文件夹中维护，Shine 再把每个文件安装到应用真正读取的位置。
- **让每台机器保留自己的参数：** 预设只声明需要哪些键，具体值由每台机器在本地提供。
- **更新前先看一眼：** 默认先查看发生了什么变化，再由你决定何时升级应用。
- **不用时只删 Shine 安装的内容：** 来源文件夹和无关文件都会保留。

## 现在就能试

- 安装 `shell/proxy`，把 `setproxy` 和 `usetproxy` 作为普通命令使用。
- 查看 Starship、Git、Vim、Ghostty 等工具已有的配置预设。
- 需要配置 Surge 或 Clash Verge Rev 时，使用已经准备好的专用流程。

```bash
shine list --available
shine install shell/proxy
shine info shell/proxy
```

完整的第一次使用流程见[快速开始](https://biulight.github.io/shine/zh-Hans/quick-start)。

## 也可以自己做

预设文件夹可以通过任意文件夹同步工具、压缩包、网络传输、版本管理工作区或手工复制到达机器，
Shine 不限定你怎样分享它。

你可以把批量重命名、图片压缩与缩放、表格整理或文档打印封装成自己的可复用命令。这些是可以
自行构建的方向，并非已内置工具；每个命令需要的应用或运行时仍要安装在对应机器上。可以从
[自定义预设指南](https://biulight.github.io/shine/zh-Hans/guides/custom-presets)开始。

Shine 也能初始化 macOS、Ubuntu 或 Windows 的选定部分，但不会接管第三方工具版本；具体边界见
[系统初始化](https://biulight.github.io/shine/zh-Hans/guides/system-init)。

## 安装

macOS 与 Linux：

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

已经安装 Rust 1.88 或更高版本时：

```bash
cargo install shine-cli
```

开发、测试、规划与发布流程以仓库根目录的 [`README.md`](../README.md)、[`AGENTS.md`](../AGENTS.md)、
[`docs/PLAN.md`](PLAN.md) 和 [`docs/kb/`](kb/) 为准。

## 许可证

MIT OR Apache-2.0
