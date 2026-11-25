---
title: 管理应用配置
sidebar_position: 2
---

# 管理应用配置

App 预设用于安装和更新应用配置。[内置 App 预设](../built-in-presets/index.md#app-预设)需要另行
安装应用本身。

## 查看与预览

```bash
shine app list
shine app info starship
shine app install starship --dry-run
```

用 `shine app list` 选择类别，再用 `shine app info <CATEGORY>` 查看各文件的状态和目标路径。
写入需要管理员权限的系统目录前，先预览安装操作。`--dry-run` 不会写入应用文件。

## 安装与更新

```bash
shine app install starship
shine update app/starship
shine upgrade app/starship
```

`update` 检查差异，`upgrade` 应用配置更新。以上命令只针对 Starship；安装的等价写法是
`shine install app/starship`。

改动前，Shine 会显示计划并请求确认，默认为否。请检查文件、必需输入和权限。自动化中可在审阅后
使用 `--yes`，但不能绕过权限、信任或安全检查。运行外部代码的预设还需要[代码授权](#生命周期钩子)。

替换已有的非受管配置文件前，会先创建 `*.shine.bak` 备份。[秘密文件](#managed-secret-files)
使用更严格的冲突规则，不会创建明文备份。重新安装会保留后续修改，包括受管 JSON 键。若要用预设
内容覆盖自己的修改，先预览，再批准：

```bash
shine app install starship --replace-managed --dry-run
shine app install starship --replace-managed
```

要移除预设不再声明且未被修改的文件，在审阅的 upgrade 命令中添加 `--prune-stale`。修改过的文件仍会保留。

### 理解配置变化 {#配置变换}

部分预设会将 JSONC 或支持的 JSON5 转为 JSON、用 `[env]` 填充 `@@VAR_NAME@@`，或只管理指定的
JSON 键。更新时比较的是安装结果与处理后的配置；受管 JSON 键之外的设置会保留。

### 刷新生成的配置 {#生成式文件与-surge-uri-订阅}

部分预设会根据订阅 URL 等输入生成文件。普通 `list`、`info` 和 `update` 不运行生成器。
`generator not evaluated` 表示尚未检查生成内容，并不代表已安装文件是最新状态。

只检查生成内容、不安装时，运行：

```bash
shine update app/surge --run-generators --diff
```

这会运行所选 App 的自动和手动生成器，可能访问网络。外部预设或 overlay 的生成器需要已有的
[信任授权](#生命周期钩子)。检查不会安装文件或运行应用后续操作；请先解决失败或未完成的检查，
再依赖其结果。要检查所有已安装 App 类别，可用 `shine update --run-generators`。

要安装新生成的内容：

```bash
shine app refresh <CATEGORY>
shine app refresh <CATEGORY> <SOURCE_FILE>
```

按需选择整个类别或单个文件。`SOURCE_FILE` 是预设列出的源文件相对路径。新增文件需要先执行
`shine app install <CATEGORY>` 才能刷新。失败时保留上次成功内容；本地修改也会保留，除非添加 `--force`。

自动生成器也可在安装和升级时运行；手动生成器在安装、显式检查或刷新时运行，不随日常升级执行。
使用 `--replace-managed` 重新安装时也可能运行它们。只有符合预设启用条件的生成器才会执行。

### 将配置接入应用 {#构建辅助资源}

预设提示需要额外设置时，运行：

```bash
shine app artifact apply <CATEGORY>
```

此操作可能修改应用设置，请按预设说明操作并审阅计划。部分预设还会在安装或升级写入文件后自动
重新加载应用；未变化的类别不会触发这些操作。请检查报告的失败。后续操作失败不会阻止其他类别
的安装或升级；需要查看预设允许显示的成功输出时，使用 `shine upgrade --verbose`。

编写这些脚本的方法见[自定义预设](./custom-presets.md#app-构建脚本运行时)。

## 授权外部代码 {#生命周期钩子}

外部钩子、生成器和 artifact 脚本运行时没有沙箱隔离，批准前请审阅代码。交互确认可以只授权本次
列出的代码，不保存信任。自动化和显式 `--run-generators` 检查需要持久信任：

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

授权覆盖类别中列出的全部代码，包括辅助文件；类别内容变化后可能需要重新审阅。`--development`
还会信任所显示本地来源中的未来代码修改，仅在明确接受这一范围时使用。信任不能解决缺失输入、
元数据不兼容或被阻止的 [App 代码检查](./custom-presets.md#执行前检查-app-代码)。

## 卸载与恢复

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
```

卸载会保留安装后被修改的文件并报告冲突，并还原安装时创建的原文件备份。`--force` 允许删除
修改过的受管内容，使用前应先通过 `--dry-run` 预览该删除操作。

卸载也会删除该类别的整个 Shine 缓存，包括额外存入的文件；未指定类别时，会清理全部 App 缓存。
请勿在缓存中存放个人文件，外部预设来源会保留。`shine uninstall app/<CATEGORY>` 使用相同的类别限定范围。

即使应用清理失败，受管文件仍可能被移除。请检查警告和最终结果，并按应用说明完成清理。预设提供
清理操作时，可按需单独运行 `shine app artifact remove <CATEGORY>`。具体效果取决于预设，请确认
哪些设置会被移除，以及哪些步骤需要手动完成。
Apply 或 remove 失败、中断时，部分设置可能已改变：修复提示的问题后，重复原命令。
不要删除安装记录或备份来绕过失败。

## 处理冲突与中断

另一项 App 操作仍在运行时，请等待它结束。若 Shine 提示审阅后文件或设置发生变化，请重新审阅计划。
安装失败时可能已有部分文件安装成功，重试前请检查报告结果。

更新导致受管文件移动时，旧文件必须未修改，新路径必须空闲；否则 Shine 保留两端现状。生成文件
移动时，批准前还应检查旧备份：备份缺失或变化可能需要手动修复，`shine app recover` 不支持恢复
这类位置迁移。

### 恢复中断的 App 操作

操作中断后可能阻塞后续改动，只读命令仍可使用。运行：

```bash
shine app recover
```

审阅恢复计划及其要求的管理员权限。只有受影响文件仍与记录状态一致时，恢复才会完成或回退原操作。
后续修改及无关 JSON 设置会保留，遇到冲突则停止。请先备份并检查指出的路径，再处理冲突。恢复文件
可能包含敏感配置，不要手动编辑或删除恢复状态。自动化可在审阅计划后使用 `shine app recover --yes`。

### 改用旧版 Shine {#upgrade-and-downgrade}

Shine 2.8 及更高版本保存的 App 安装记录无法由 2.7.x 及更早版本读取。改用旧版前：

1. 备份需要保留的配置，使用新版程序处理受保护文件和待恢复操作，再预览并完成 App 卸载。
2. 若卸载中断，重新执行同一卸载命令，完成清理。
3. 更换程序，使用旧版支持的预设重新安装。

安装旧程序、重新封存加密源文件或使用 `--replace-managed` 都不能转换记录。不要删除记录或修改
其版本号来绕过检查。

### 迁移旧预设 {#将旧版-app-metadata-迁移到-shine-2}

若 Shine 提示外部预设或 overlay 的元数据不兼容，先执行 `shine preset migrate --dry-run` 预览，
再审阅并运行 `shine preset migrate`。迁移会备份元数据并保留自定义配置文件。安装前先解决需要手动
修改的项目，详见[迁移 1.x 来源](./custom-presets.md#迁移-1x-来源)。

## 管理秘密文件 {#managed-secret-files}

App 预设可以安装 workspace 密文，并管理服务所需的解密文件。明文不得进入预设仓库，解密身份应
留在执行解密的机器上。编写此类预设时，参见[声明受管秘密文件](./custom-presets.md#app-secret-files)。

启用 `auto_decrypt` 后，安装与升级会在批准后、修改该类别目标文件前完成所需解密；未变化的文件
跳过解密。只读命令和 diff 只显示状态，不解密或展示内容。启用 SSH broker 的会话由本机完成解密
及硬件授权，否则使用安装机器配置的后端。broker 失败不会回退到其他后端。远程使用这些预设前请
升级 SSH 两端，参见 [SSH 文件授权](ssh-transfer.md#file-secret-broker)。

### 准备服务读取权限

Unix 默认使用安装用户的所有权和 `0600`。其他服务账号可通过预设的 `unix_uid`、`unix_gid` 及
`unix_mode`（`0400` 或 `0600`）可选字段指定。请以有权创建目标、设置该所有权的账号安装。对于其他非 root
所有者，应预先创建所有父目录，通过 Unix 权限位为该 UID、选定 GID 或其他用户提供遍历（`x`）
权限。仅通过附加组或 ACL 获得访问还不够；Shine 不会修改已有目录权限。Windows 上秘密文件仅供
当前用户使用，应省略 Unix 字段。安装前请审阅权限。

### 处理已有文件与移除

- 非受管目标只有与解密结果内容一致时才会被接管。内容不同即冲突，`--replace-managed` 也不能
  覆盖，不会创建明文 `.shine.bak`。在 Windows 上，已有文件的所有者必须是运行安装的用户。
  请先禁用权限继承，只保留该用户的一条允许访问条目，再重试。
- 后续本地修改默认保留，只有显式替换才会覆盖。卸载会移除未修改的秘密文件，保留本地修改。
  关闭自动解密会暂停创建和更新，但已有文件仍受管理。删除声明后，需用 `upgrade --prune-stale` 移除
  未修改的旧目标；目标迁移需先处理旧安装。
- 写入中断后，审阅并执行 `shine app recover`。不要删除 Shine 的私有秘密文件状态或密钥；即使
  内容看起来未变，恢复仍可能需要人工检查。

关闭自动解密时，可在安装后的 workspace 中运行 `shine env workspace export --format files`，
明确选择模式和文件组。手动导出拒绝已有文件，默认使用私有权限，也不会将其纳入 App 管理。
请自行设置服务读取权限；后续自动安装可以接管内容相同的文件。

容器运行时仍需明文。Shine 不会启动或重启容器：替换密码文件后，需重建依赖更新文件挂载的容器；
卸载必要配置前，应先停止依赖它的服务。

## 选择内置预设 {#按应用配置}

<span id="surge-uri-订阅" />
<span id="clash-verge-rev" />

可选配置及其前置条件、定制和移除步骤，见[内置 App 预设](../built-in-presets/index.md#app-预设)。
