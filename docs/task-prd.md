# 个人快捷任务 PRD

## 1. 背景

用户在日常开发中经常重复执行一些较长、易输错、带本地路径或远端路径的命令，例如：

```bash
rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
lsof -i :3000
kill <pid>
```

这些命令高度个人化、项目化、环境化，不适合作为内置 preset 随二进制发布；但让用户每次都
手写命令，或为了一个个人快捷命令维护完整 shell preset，也显得过重。

`shine task` 提供一个轻量的个人任务系统，让用户把常用命令保存到 shine 管理的运行时状态中，
并通过稳定入口执行。

## 2. 产品目标

- 允许用户保存、查看、执行、删除常用命令。
- 正式模型使用 `shine task ...`，让任务能力集中在一个清晰命名空间下。
- 提供顶层便捷入口 `shine run <name>`，作为 `shine task run <name>` 的 alias。
- 任务存储在 shine runtime state 中，并随 `SHINE_CONFIG_DIR` 生效，方便测试和隔离。
- 执行时保持透明：用户能查看任务实际命令，运行前能看到即将执行的命令。
- 避免动态顶层命令污染；不支持 `shine <task-name>` 直接执行。

## 3. 非目标

- MVP 不做完整工作流编排，不支持 task 依赖、并发、条件判断。
- MVP 不做交互式 TUI。
- MVP 不自动解析或安全改写 shell 命令。
- MVP 不支持模板参数，例如 `{port}`、`{env}`。
- MVP 不支持任务描述、创建时间、更新时间等元数据。
- MVP 不替代 shell alias、Makefile、justfile；它解决的是“个人高频命令保存与快速执行”。
- MVP 不把 task 安装成 `~/.shine/bin/<name>` 下的独立 shell 命令。

## 4. 核心概念

### 4.1 Task

Task 是由用户保存的命令条目，至少包含：

- `name`：用户指定的稳定标识符。
- `command`：按 argv 数组保存的命令与参数。

Task 可以通过 `task save --cwd <DIR>` 显式保存固定工作目录。未设置 `cwd`（包括旧版 task）时，
执行仍使用用户调用 `shine task run` 或 `shine run` 时的当前工作目录。

### 4.2 正式入口与便捷 alias

`shine task` 是正式产品模型，负责完整的保存、查看、执行和删除能力。

`shine run <name>` 只是 `shine task run <name>` 的便捷 alias。它不引入独立语义，也不拥有单独
存储。

### 4.3 命令存储形态

命令按 argv 数组保存，而不是保存单个 shell 字符串。执行时默认不通过 `sh -c`。

如果用户确实需要 shell 管道、重定向、变量展开等语法，应显式保存 shell 调用：

```bash
shine task save kill-port-3000 -- sh -c 'lsof -ti :3000 | xargs kill'
```

**平台说明（决策）**：命令按 argv 直接执行（`std::process::Command`，不经 shell），因此任何真实
可执行文件在所有平台都可运行；但 `sh -c '...'` 这类 shell 语法逃生舱是 **Unix 专属**，Windows
无 `sh`。argv 可跨平台存储，但执行语义不同。此限制仅在文档中标注，MVP 不做特殊处理。

## 5. 命令设计

### 5.1 保存任务

```text
shine task save <NAME> [--force] [--cwd <DIR>] -- <COMMAND> [ARGS]...
```

示例：

```bash
shine task save deploy-keystone -- rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
shine task save port-3000 -- lsof -i :3000
shine task save build --cwd . -- cargo build
```

行为：

- `<NAME>` 必须是稳定标识符，允许字符为字母、数字、点、短横线、下划线。
- `<NAME>` 必须以字母或数字开头。
- `--` 后的内容作为 argv 数组保存。
- `--cwd` 是可选的；`~` 会按有效用户 home 展开，相对路径基于保存时当前目录解析。
- 指定的 cwd 必须已经存在且是目录，并以规范化绝对路径保存。
- 未指定 cwd 时不记录目录，执行时继续使用调用者当前目录。
- 如果任务已存在，默认报错。
- `--force` 覆盖已有任务；未再次传入 `--cwd` 会清除旧任务的固定目录。
- 保存成功后打印任务名和完整命令。

建议输出：

```text
Saved task deploy-keystone
rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
```

### 5.2 执行任务

正式入口：

```text
shine task run <NAME> [-- <EXTRA_ARGS>...]
```

便捷入口：

```text
shine run <NAME> [-- <EXTRA_ARGS>...]
```

行为：

- 有固定 cwd 时在该目录执行；否则在当前工作目录执行。
- 子进程继承当前 stdin/stdout/stderr。
- 子进程继承当前环境变量。
- `shine` 返回任务命令的退出码。
- 执行前默认打印一行任务信息。
- `--` 后的额外参数以简单追加方式拼到已保存 argv 后面。

建议输出：

```text
Running deploy-keystone: rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
```

额外参数示例：

```bash
shine task save lsof-port -- lsof -ti
shine task run lsof-port -- :3000
```

实际执行：

```bash
lsof -ti :3000
```

### 5.3 查看任务列表

```text
shine task list
```

建议输出：

```text
Saved Tasks
deploy-keystone  rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
port-3000        lsof -i :3000
```

如果为空：

```text
No saved tasks yet. Run `shine task save <name> -- <command...>`.
```

### 5.4 查看任务详情

```text
shine task info <NAME>
```

建议输出：

```text
Task       deploy-keystone
Command    rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/
```

### 5.5 删除任务

```text
shine task delete <NAME>
```

行为：

- 删除存在的任务。
- 任务不存在时报错。
- MVP 不需要确认提示，因为删除只影响 shine 自己的任务记录。

建议输出：

```text
Deleted task deploy-keystone
```

## 6. `shine run` alias 规则

`shine run <NAME>` 等价于 `shine task run <NAME>`。

如果未来 `shine` 已经存在或新增了正式顶层 `run` 命令并产生语义冲突，则：

- 禁用当前 alias。
- 执行 `shine run ...` 时返回清晰错误。
- 错误中列出冲突的完整命令和替代命令。

示例：

```text
`shine run` is disabled because it conflicts with a built-in command.

Use the full task command instead:
  shine task run deploy-keystone

Conflicting command:
  shine run
```

当前实现中，如果 `run` 没有其他正式语义，可以注册为顶层命令并在代码注释中明确它是 task
alias，不是独立产品模型。

**决策**：MVP 已按此方式注册顶层 `run`（无冲突），并在代码注释标明它是 task alias。上述冲突
禁用分支在 MVP **不实现**——仅当未来真正新增正式 `run` 语义时再补。

## 7. 存储设计

新增任务存储文件：

```text
<shine_dir>/tasks.toml
```

示例：

```toml
[tasks.deploy-keystone]
command = ["rsync", "-avz", "dist/", "marqueeio.develop:/var/www/keystone/alex/"]

[tasks.port-3000]
command = ["lsof", "-i", ":3000"]

[tasks.build]
command = ["cargo", "build"]
cwd = "/Users/alice/src/project"
```

设计选择：

- task 属于运行时用户状态，不属于 embedded preset。
- task 文件随 `SHINE_CONFIG_DIR` 切换，测试时写入隔离目录。
- 命令按 argv 数组保存，保留参数边界。
- `cwd` 是可选的规范化绝对路径；缺失时保持动态当前目录语义。
- 执行时使用进程 API，不默认通过 shell。
- 如果需要 shell 语法，由用户显式保存 `sh -c ...` 或对应平台 shell。

MVP 可以直接读写整个 `tasks.toml`，不要求保留用户手写注释。后续如果用户编辑该文件的场景变多，
再考虑注释保留策略。

## 8. 错误处理

任务名不存在：

```text
Task not found: deploy-docs

Run `shine task list` to see saved tasks.
```

保存空命令：

```text
No command provided.

Usage:
  shine task save <name> -- <command...>
```

任务名非法：

```text
Invalid task name: deploy docs

Task names may contain letters, numbers, dots, dashes, and underscores.
```

保存时已存在：

```text
Task already exists: deploy-keystone

Use `--force` to replace it.
```

执行失败，命令不存在：

```text
Failed to run task deploy-keystone: command not found: rsync
```

子命令非零退出：

- `shine` 以同样 exit code 退出。
- 不额外包装成 anyhow 错误，以保留命令本身语义。

## 9. 安全与透明性

- 不默认使用 `shell -c`。
- `shine task info` 必须能显示完整 argv 渲染后的命令。
- 有固定 cwd 时，`task save`、`task info` 和运行提示显示该目录（home 下路径缩写为 `~`）。
- **命令展示使用 shell 引用（决策）**：`info`/`list`/`run` 把 argv 渲染回命令行时，对含空格或
  shell 特殊字符的参数加单引号（内部 `'` 转义为 `'\''`），路径/URL 等安全字符不加引号。目的是让
  展示的命令可直接复制粘贴执行，例如 `sh -c 'lsof -ti :3000 | xargs kill'` 会带引号显示。
- `shine task run` 和 `shine run` 执行前显示实际命令。
- 不做危险命令拦截，例如 `rm -rf`。不可靠的拦截会制造虚假的安全感。
- 后续可增加 `--quiet` 跳过运行提示；MVP 不需要。

## 10. 与现有系统关系

- 不复用 shell preset 的安装机制作为 MVP 核心。
- 不改变 embedded presets、app presets、sys presets 的语义。
- 不改动 shell profile sentinel 逻辑。
- `shine task` 是 runtime/user state；`shine shell` 是 preset/install state。
- 后续可扩展 `shine task install <NAME>`，把任务生成成 `~/.shine/bin/<name>` 下的可执行 shim，
  允许用户直接执行 `deploy-keystone`。该扩展需要单独处理 PATH、冲突检测和 uninstall 语义，不属于
  MVP。

## 11. 推荐 MVP 命令集

```text
shine task save <NAME> [--force] [--cwd <DIR>] -- <COMMAND> [ARGS]...
shine task run <NAME> [-- <EXTRA_ARGS>...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- <EXTRA_ARGS>...]
```

## 12. 验收场景

- 保存一个新任务后，`shine task list` 能显示任务名和命令。
- 保存一个新任务后，`shine task info <NAME>` 能显示完整命令。
- `shine task run <NAME>` 能执行成功命令并返回 0。
- `shine task run <NAME>` 能透传失败命令的 exit code。
- `shine task run <NAME> -- <EXTRA_ARGS>...` 能把额外参数追加到保存的 argv 后。
- `shine run <NAME>` 与 `shine task run <NAME>` 行为一致。
- `shine task run <MISSING>` 返回清晰错误，并提示使用 `shine task list`。
- `shine task save <EXISTING>` 默认报错。
- `shine task save <EXISTING> --force -- ...` 覆盖旧命令。
- `shine task delete <NAME>` 删除任务，删除后不可再运行。
- 使用 `SHINE_CONFIG_DIR` 时，任务文件写入隔离目录。
- 保存带空格参数的命令时，argv 能正确保留参数边界。
- 使用 `--cwd` 保存后，从其他目录运行仍在固定目录中执行。
- 未使用 `--cwd` 的新旧任务仍在调用目录执行。
- 保存不存在或非目录的 cwd 时失败；保存后目录消失时运行给出明确 cwd 错误。

## 13. 后续方向

- `shine task rename <OLD> <NEW>`：重命名任务。
- `shine task edit <NAME>`：编辑任务命令。
- `shine task install <NAME>`：安装成 `~/.shine/bin/<name>` 下的独立命令。
- 支持 task 描述、环境变量覆盖。
- 支持模板参数，例如 `shine task run kill-port --port 3000`。
- 支持从当前 shell history 中选择并保存任务。
