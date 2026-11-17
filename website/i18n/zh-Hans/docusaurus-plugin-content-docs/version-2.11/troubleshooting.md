---
title: 故障排查
sidebar_position: 6
---

# 故障排查

先运行 `shine --version` 记录版本，再使用 `shine list`、`shine info <TARGET>` 或对应命令的 `--dry-run` 收集状态。

## 安装后找不到命令

Shell 预设的命令入口位于 `~/.shine/bin/`。安装完成后需要打开新终端，或重新加载 shell profile：

```bash
source ~/.zshrc
# 或 source ~/.bashrc
```

然后检查：

```bash
shine list
shine info shell/proxy
```

命令入口缺失时，先用 `shine info shell/<CATEGORY>` 检查详情。确认本地修改后，执行
`shine shell install <CATEGORY> --replace-managed` 重建受管文件和命令入口。

## 应用配置显示为用户修改

Shine 默认保留安装后被修改过的文件。先查看差异：

```bash
shine info app/starship --diff
```

需要采用预设版本时运行 `shine app install starship --replace-managed`。卸载时，只有明确要删除这些本地修改才使用 `shine app uninstall starship --force`；先运行相同命令并加上 `--dry-run`。

## 使用的不是预期预设

外部目录、项目配置和环境变量可能改变当前来源。检查命令输出中的 `Preset Source` 与
`Presets Overlay`，并按[配置参考](./reference/configuration.md)核对优先级。

如果只想检查账户配置，请先切换到没有 `shine.config.toml` 的项目之外，再查看来源。
`SHINE_CONFIG_DIR` 只切换账户配置与状态目录，不会关闭当前项目配置的查找。

## 修改 env 后配置没有变化

`shine env set` 更新变量，但已安装模板不会自动重写。运行：

| 操作 | 命令 |
| --- | --- |
| 详细检查待更新内容 | `shine update --verbose` |
| 更新配置并显示详细输出 | `shine upgrade --verbose` |

如果使用项目 `shine.config.toml`、项目 `shine.env.toml` 或 overlay，请确认当前工作目录和覆盖优先级。

如果问题表现为私有域名或 `192.168.x.x` 地址仍然被终端代理接管，可继续参考知识库中的
[终端代理误拦截 ZeroTier 私有域名的排查与修复](https://blog.biulight.top/timeline/knowledge/terminal-proxy-no-proxy-zerotier)。

## 升级到 0.40 后提示旧配置文件

项目中的 `config.toml` 和 `.env.toml` 已不再作为 Shine 配置读取，请分别改名为 `shine.config.toml` 和 `shine.env.toml`。

若提示检测到全局 `~/.shine/env.toml`，不要直接删除仍在使用的值。将它移动为 `~/.shine/shine.env.toml`；若新文件已经存在，先手动合并并检查重复键。也可以在升级前先用 v0.39 运行一次配置加载，让旧版本完成自动迁移。

## 生成式 App 文件刷新失败

先确认类别已经安装，`SOURCE_FILE` 使用 `[[files]].source` 的相对路径：

```bash
shine app info <CATEGORY>
shine env list
shine app refresh <CATEGORY> <SOURCE_FILE>
```

按该预设的要求准备运行时和环境输入。Surge 特有的订阅错误见[刷新订阅](./built-in-presets/surge.md#刷新订阅)。

如果目标文件已被修改，先查看差异；只有确定要覆盖本地修改时才加 `--force`。
刷新失败会保留上次成功生成的文件。

自动与手动生成的区别见[刷新生成的配置](./guides/app-presets.md#生成式文件与-surge-uri-订阅)。
提交问题前，请检查报错输出，避免附上秘密。

## `shine preset pull` 拒绝更新来源

先区分来源类型：通过 `shine preset overlay link --git` 创建的覆盖层是 Shine 管理的镜像，
拉取时会丢弃镜像内的本地修改和额外文件。请在上游仓库修改，不要直接编辑本机镜像；
详见[Git 覆盖层](./guides/custom-presets.md)。

对于自己维护的外部来源或手动链接的覆盖层，`shine preset pull` 只对干净、已设置 upstream
的普通分支执行快进更新。进入错误信息显示的仓库并检查：

```bash
git status
git branch --show-current
git branch -vv
git pull --ff-only
```

请自行提交、stash 或处理本地改动和分支分歧，再重新运行 `shine preset pull`。对于这些手动维护的来源，Shine 不会自动丢弃改动或解决冲突。若提示找不到 Git，请先安装 Git 并确认 `git` 在 `PATH` 中；非 Git 预设目录被跳过属于正常行为。

## 系统初始化前想确认影响

```bash
shine sys info <ITEM>
shine sys bootstrap --dry-run
shine sys uninstall <ITEM> --dry-run
```

使用 `shine sys list` 和 `shine sys info` 确认当前版本提供的项目及安装要求。

若旧版本在 shell profile 中留下冲突标记，请先手动处理，再运行
`shine sys bootstrap --dry-run` 或对应的 `upgrade`。

## 任务运行结果和手动执行不同

保存的任务需要管道、重定向、变量展开或通配符时，请显式调用 Shell：

```bash
shine task save kill-port -- sh -c 'lsof -ti :3000 | xargs kill'
```

额外参数会追加到已保存命令末尾：

```bash
shine task run my-task -- --verbose
```

## SSH 文件传输不可用

`shine local` 只能在 `shine ssh` 打开的远端 shell 中使用。若提示缺少 `SHINE_SSH_SESSION`、`SHINE_SSH_TOKEN` 或 `SHINE_SSH_REMOTE_SOCK`，请退出后重新用 `shine ssh <HOST>` 进入。

远端也需要能运行兼容的 `shine local`。先检查：

```bash
shine local status
which shine
shine --version
```

传输前优先预览路径和覆盖行为：

```bash
shine local download ./remote.log ./remote.log --dry-run
shine local upload ./local.log /tmp/local.log --dry-run
```

使用 rsync 时默认跳过已有文件，添加 `--force` 后才允许覆盖；目录目标已存在时，`--force` 表示合并写入。
SCP（包括自动回退）即使不带 `--force` 也可能覆盖文件。传输前请预览路径，并备份目标中需要保留的文件。

## 没有显示维护提醒

运行 `shine state check` 可立即检查。自动提醒可能在后续交互命令中出现，重定向或自动化命令会
跳过它们。若检查无法完成，请确认当前用户能访问系统服务管理器后重试。清理旧 HTTP 服务使用
`shine clear serve`。

## 自动更新检查失败

网络或 GitHub API 不可用时，Shine 会跳过版本检查并继续执行原命令。恢复网络后可绕过 24 小时缓存重新检查：

```bash
shine update --refresh-release
```
