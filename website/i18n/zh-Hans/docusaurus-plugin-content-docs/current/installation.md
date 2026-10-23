---
title: 安装与升级
sidebar_position: 2
---

# 安装与升级

Shine 支持 macOS、Linux 和 Windows。官方安装脚本从 GitHub Releases 下载当前平台的二进制文件。

## macOS 与 Linux

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

默认安装到 `~/.local/bin/shine`，安装脚本不会修改 shell 配置。确认 `~/.local/bin` 已在 `PATH` 中，然后验证：

```bash
shine --version
```

如需指定位置或版本：

```bash
SHINE_INSTALL_DIR=/custom/bin sh install.sh
SHINE_VERSION=2.7.0 sh install.sh
```

## Windows PowerShell

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

默认安装到 `%LOCALAPPDATA%\Programs\shine\shine.exe`，不会修改用户 `PATH`。如需指定位置或版本：

```powershell
$env:SHINE_INSTALL_DIR = "$env:USERPROFILE\bin"; .\install.ps1
$env:SHINE_VERSION = "2.7.0"; .\install.ps1
```

## 使用 cargo-binstall 安装

已安装 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 时，可从官方 GitHub Releases
下载预编译的 Shine 二进制文件：

```bash
cargo binstall shine-cli
shine --version
```

包名是 `shine-cli`，安装后的命令是 `shine`。默认安装到 `$CARGO_HOME/bin`
（通常为 `~/.cargo/bin`），请确认该目录已加入 `PATH`。

官方二进制文件覆盖 macOS、使用 glibc 的 Linux 和 Windows（MSVC 构建），均提供 x86_64 和
aarch64 两种架构。暂不提供 Linux musl 二进制文件。找不到适用的二进制文件时，cargo-binstall
可能尝试第三方二进制文件或从源码编译。如需限定为官方预编译二进制文件，使用：

```bash
cargo binstall shine-cli --strategies crate-meta-data
```

再次运行 `cargo binstall shine-cli` 可通过 cargo-binstall 升级。滚动的 `preview` 通道请使用
`shine self upgrade --channel preview`。

## 从源码安装

已安装 Rust **1.90 或更高版本**时，可从 crates.io 安装：

```bash
cargo install shine-cli
```

若在 Shine 源码仓库中构建，运行 `cargo build --release`；二进制文件位于
`target/release/shine`。

## 升级 Shine

安装后可由 Shine 下载稳定版或 preview 版：

```bash
shine self upgrade
shine self upgrade --channel stable
shine self upgrade --channel preview
```

显式传入 `--channel stable` 也允许重装相同版本或安装较旧的稳定版。
成功消息会报告安装后和此前的版本，不会一律称为版本升级。

`shine self upgrade` 失败时返回退出码 1，并在 stderr 中保留失败上下文和底层原因，
便于区分连接、归档文件和安装错误。

`preview` 是持续滚动的预发布通道，不参与日常自动更新检查。运行 `shine update` 可以同时检查已安装配置和稳定版程序更新。

稳定版通过 `latest` 和 stable 通道提供。从既有 1.x 安装升级前，请先阅读
[从 Shine 1.x 升级](./guides/upgrade-to-2.md)了解兼容性变化。

Shine 2.5 新增[临时机密文件](./guides/environment.md#workspace-secret-files)，可为构建过程提供凭据。
既有 version 2 工作区继续受支持。采用 workspace version 3 前，请将所有读取它的客户端升级到
Shine 2.5 或更高版本；旧客户端会拒绝新格式。Windows 上新建环境代理必须使用不含可执行文件
扩展名的名称（例如 `gh`，而非 `gh.exe`）；已安装的带后缀代理仍可卸载。

从 2.3.x 或更早版本升级时，请先阅读[环境分层的升级与回退步骤](./guides/environment.md#env-分层变更的人工升级与回退)。
2.4.0 调整了同名变量的层级优先级，并用显式目标选项替代环境写入的 `--force`。
请保留私有备份，并在重要项目中运行 `shine env explain KEY` 检查实际来源。

`shine self install --dest <PATH>` 按当前目录解析相对路径，并记录绝对目标路径，因此之后从任何目录升级时都会同步同一份已安装副本。
若安装目录不在 PATH 中，输出的 POSIX shell 示例会保留目录名中的特殊字符。
`self install` 的结果路径会转义终端控制字符；目录包含控制字符时，会要求使用原目录手动设置 PATH，
不生成可复制示例。若升级成功但同步另一份安装副本失败，Shine 会向 stderr 输出包含原因的警告；升级仍返回成功。
同步结果路径和警告详情会转义终端控制字符，已记录的目标保持不变。

在 Unix 系统上，如果 `shine self install` 要把二进制文件复制到当前用户不可写的位置，Shine 会交互式请求授权，并通过 `sudo` 完成安装。`shine self upgrade` 成功后同步另一个已记录的安装目标时，也会采用相同方式。不过，如果当前运行的 Shine 本身位于受保护目录，Shine 暂时无法自动提权替换该二进制文件；请改为安装并运行用户可写位置中的 Shine。Windows 在受保护位置安装或升级时，仍需使用具备相应权限的终端。

## 升级到 Shine 2.7

从 2.6 或更早版本升级时，请在更新外部 Preset 或自动化前检查以下兼容性变化。
源码构建现在要求 Rust 1.90；使用预编译二进制无需安装 Rust。

1. **转换旧 Preset 环境输入。** Shell Bun 命令及 App 的 generator、hook、artifact 要求结构化
   `env`，包含 `source`、可选 `target` 和 `sensitivity`；旧字符串数组会被拒绝。
   先为来源保留私有备份，再用 Shine 2.7 预览并应用转换：

   ```bash
   shine preset migrate ./my-presets --dry-run
   shine preset migrate ./my-presets
   shine preset validate ./my-presets
   shine preset lint ./my-presets
   ```

   迁移需要旧声明中的显式敏感性；缺失或冲突时须人工修正。Git 管理的 overlay 要在上游
   checkout 中迁移，再重新 pull。迁移会备份元数据，但不会执行脚本、解密值或更新信任。
   正常 install/upgrade 前，请重新审阅变化后的外部快照。
   详见[运行时环境输入](./guides/custom-presets.md#声明运行时环境输入)。
2. **审阅 App 代码诊断。** 受支持的 Bun 代码及本地 JS/TS 依赖会在执行前检查。
   确定的声明差异和分析失败会阻止相关操作，即使已有信任或使用 `--yes` 也一样。
   请修正声明或代码；添加 unrestricted-code 元数据不能绕过检查。覆盖缺口仍只作提示，
   不代表沙箱隔离或安全证明。普通文本会汇总重复缺口，详细视图和 lint JSON 保留诊断。
   详见 [App 代码检查](./guides/custom-presets.md#执行前检查-app-代码)。
3. **更新报告消费者并重新取得批准。** 升级后运行 `shine preset schema`。
   生命周期 Plan／批准／审阅及作者 Plan 报告使用 schema 3；lint 和 fixture 报告使用
   schema 2，fixture 输入 schema 1 继续受支持。应重新审阅，不能复用旧批准。
   文本汇总后，`--deny-warnings` 仍计算每条告警。
   使用 `shine-core` 的 Rust 集成还需适配公开结构体新增的分析字段，例如 `PlanV1` 和
   `InspectionReportV1`：更新直接构造结构体及穷举字段的模式匹配代码，再重新编译。
   序列化 schema 的兼容性不代表 Rust 源码兼容。

已安装的 launcher、receipt、恢复记录及 `shine env run --with SOURCE=TARGET` 保留原有格式。
降级二进制不会撤销元数据转换；如需回到旧客户端，请保留原始来源或迁移备份。

## 升级到 Shine 2.6

Shine 2.6 从 `uninstall`、`app uninstall` 和 `shell uninstall` 中移除了 `--purge`。
请从脚本中删除该选项；仍含此选项的命令会在加载配置前失败。
卸载现在默认将内部缓存清理纳入待审批的 Plan。先用 `--dry-run` 预览，尤其是在删除整个
App 分类或所有 Shell 预设前。外部预设源会保留。清理范围和文件保留规则见
[App 卸载](./guides/app-presets.md)和 [Shell 卸载](./guides/shell-presets.md)。

Shine 2.6 还新增了持久化机密文件导出：
`shine env workspace export --format files --mode <MODE> --file-group <GROUP>`。
与 `env run` 提供的临时文件不同，导出的明文会留在磁盘上，直到你自行删除。
使用前请阅读[机密文件指南](./guides/environment.md#workspace-secret-files)。

## 清理旧版本的本地 HTTP 服务

2.4.0 版本移除了 HTTP 服务能力和脚本变量 `SHINE_APP_HTTP_DIR`。Surge 和 Clash Verge 规则可使用
本地文件；应用 artifact 的持久输出可写入 `SHINE_STATE_DIR` 或 `SHINE_CACHE_DIR`。

如果曾安装旧版后台服务，升级到 2.4.0 后可执行以下命令清理：

```bash
shine clear serve
```

`shine clear serve` 会停止旧服务并删除其用户服务注册；整个 `shine serve` 命令已移除。
即使旧版 Shine 配置无法加载，清理命令仍可运行，无需找回旧二进制或指定原来的 `--config-dir`。
请以安装服务的用户运行；成功卸载后重复执行也是安全的。系统服务管理器报错时，命令会报告
失败，便于修复原因后重试。

普通交互命令发现遗留服务后会提醒你处理。也可执行 `shine state check` 立即检查；
此命令只报告维护事项，即使配置无法读取也可运行。

升级前，也可使用旧二进制的 `shine serve uninstall` 移除服务。

升级不会删除旧运行时目录 `http/` 下的文件。请保留仍需使用的文件，并先更新应用中的引用再删除。

下一步：[完成第一次预设安装](./quick-start.md)。
