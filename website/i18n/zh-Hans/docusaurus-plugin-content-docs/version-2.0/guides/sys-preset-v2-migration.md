---
title: 将系统预设迁移到 v2
sidebar_position: 4
---

# 将系统预设迁移到 v2

本页面向维护 **v1 自定义系统预设** 的用户。如果你只使用 Shine 2 内置的系统预设，无需手动迁移，请直接阅读[初始化与管理系统](./system-init.md)。

迁移修改的是预设目录里的 `shine.toml` 和相关脚本，不是已安装的软件。不要删除或手动改写 `~/.shine/sys-manifest.toml` 中的执行记录。

下面以 `./my-presets/sys/macos/` 中的 Neovim 安装项为例。请替换成你的实际路径；Ubuntu 使用 `sys/ubuntu/` 和 `--platform linux`，Windows 使用 `sys/windows/` 和 `--platform windows`。示例中的 Homebrew 安装方式只适用于 macOS。

## 1. 找到需要修改的预设

在修改前，先用版本控制或文件副本保留旧预设，再检查它：

```bash
shine preset migrate ./my-presets/sys/macos --dry-run
```

如果报告 `sys_v1_manual_migration_required` 并返回非零退出码，表示需要按下面的步骤手动迁移。Shine 不会自动把旧的 `init.sh` 或 `init.ps1` 拆成多个安装脚本；重复运行迁移命令不会完成这项工作。

不知道当前使用哪份预设时，运行 `shine preset migrate --dry-run` 查看报告中的路径。若使用 Shine 管理的 Git overlay，请修改它的上游工作副本，不要直接编辑本地镜像。

## 2. 先迁移一个安装项

假设旧的 `init.sh` 通过一个分支调用 `brew install neovim`。在 v2 中，这个安装步骤直接写进 `sys/macos/shine.toml`：

```toml
version = 2
description = "My macOS development tools."
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install Neovim with Homebrew."
permissions = { schema_version = 1 }

[items.detect]
kind = "command"
command = "nvim"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew"
package = "neovim"

[profiles.recommended]
items = ["neovim"]
```

这是只包含一个安装项的完整示例。迁移已有文件时，保留其他安装项及其 ID，并逐项转换，不要用示例覆盖整份配置。

- `detect` 告诉 Shine 如何检查软件是否已经存在；这里检查 `nvim` 命令。
- `install` 告诉 Shine 如何安装缺失的软件；这里使用 Homebrew，实际安装前需确保 Homebrew 可用。
- `permissions` 是每个安装项必需的权限声明。仅使用固定包管理器、没有自定义代码的这个示例可使用上述简短声明。
- `[profiles.recommended]` 选择该配置组包含的安装项。保留原有配置组时，检查其中的 ID 仍然有效。

Ubuntu 和 Windows 应使用对应的 `apt` 或 `winget` provider，并核对该包管理器中的实际包名；不要直接照搬 Homebrew 的包名。

### 安装需要自定义脚本时

如果一个安装项无法用包管理器完成，把旧脚本中属于它的逻辑移到 `install/<item>.sh` 或 `install/<item>.ps1`，并将该项的 `install` 改为 `kind = "script"`、`path = "install/<item>.sh"`（Windows 使用 `.ps1`）。

脚本只负责这一个安装项，用普通退出码报告成功或失败，并保留对应的 `detect`。根据脚本实际行为补充执行路径、命令、网络及其他所需权限，不能直接沿用上面只有 `schema_version` 的声明。具体字段见[声明权限](./custom-presets.md#声明权限)。

全部安装项迁移完成后，移除旧的统一入口及旧状态输出、更新检查逻辑。第三方软件的更新交给其包管理器或上游工具，`shine sys bootstrap` 只负责确保软件已安装。

## 3. 按需迁移 Shell 配置

旧预设没有修改 Shell 配置时，可跳过这一步。

把某个软件专属的 PATH、环境变量、别名或初始化命令放到该安装项的 `[[items.shell]]` 中。较长的脚本可放在 `profile/<item>.sh` 或 `.ps1`，再用 `fragment` 引用。`profile/base.pre.*` 和 `profile/base.post.*` 只保留操作系统通用内容。

配置形式与示例见[编写系统初始化项](./custom-presets.md#编写系统-bootstrap-item)。迁移后检查每份软件专属配置都归属于对应安装项，避免仍在公共文件中重复执行。

## 4. 验证本地文件

```bash
shine preset validate ./my-presets/sys/macos
shine preset plan ./my-presets/sys/macos --platform macos
```

先修复 `validate` 报告的错误，例如缺失文件、权限声明或无效配置。然后查看 `plan` 中的安装目标、所需权限和 Shell 配置步骤，确认它们符合预期。

这两条命令不会安装软件或执行预设脚本。`preset plan` 使用模拟环境，不是本机安装预览；缺少模拟环境中的信任、命令或管理员条件也可能使它报告阻塞。按报告区分配置错误与环境要求，不要为了让报告通过而扩大权限。

## 5. 让 Shine 使用迁移后的预设

如果你修改的就是当前已配置的来源，无需重新关联。若在另一个工作副本中迁移，请根据仓库用途选择一种方式，传入包含 `sys/` 的根目录，而不是 `sys/macos/`：

完整预设仓库，设为外部预设来源：

```bash
shine preset link ./my-presets
```

若仓库仅覆盖部分内置预设，则改用 overlay：

```bash
shine preset overlay link ./my-presets
```

所选命令会替换对应的来源设置。使用 Git 管理的 overlay 时，应先将上游修改发布到所配置的分支，再运行 `shine preset pull`。更多说明见[自定义预设](./custom-presets.md)。

在目标操作系统上检查 Shine 实际读取到的安装项：

```bash
shine sys list
shine sys info neovim
shine sys bootstrap neovim --dry-run
```

确认列表和详情包含你修改的安装项，预览中的安装命令与 Shell 配置符合预期。若仍显示旧内容，先检查来源路径及 overlay 覆盖关系。

仅使用上面包管理器示例的安装项无需额外授予代码信任。如果你加入了外部安装脚本或可执行 Shell 内容（如 `eval`、`source`、fragment 或公共 profile 脚本），先审阅代码，再检查并授予对应安装项的信任：

```bash
shine trust inspect sys/neovim
shine trust grant sys/neovim
```

权限声明说明预设需要做什么，授予信任表示你已审阅并允许执行相关代码。代码或权限变化后需要重新审阅；授权后再运行一次安装预览。

本地验证无错误、实际来源正确且安装预览符合预期后，迁移准备完成。需要实际安装时运行 `shine sys bootstrap neovim`，审阅计划后确认执行；后续操作见[初始化与管理系统](./system-init.md)。
