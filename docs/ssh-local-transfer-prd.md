# SSH 会话内本机文件传输 PRD

> **传输实现已更新（见 [ADR 0011](kb/decisions/0011-ssh-local-transfer-rsync-scp.md)）。**
> 本文档记录了产品目标与最初的设计（§9 的自定义字节流协议、§5/§6 的目录 tar 打包与
> 覆盖语义）。当前实现改为：`shine ssh` 建立的反向隧道只承载**控制 + 日志转发**通道，
> 远端发送一个 `Transfer` 请求，由**本机**运行 `rsync`（默认）/ `scp`（`--scp` 或
> 自动回退）完成传输并把输出回传远端终端；本机通过 ControlMaster 复用已认证连接，避免
> 二次认证。通配符、增量、目录/软链接/权限均由 rsync/scp 原生处理。产品目标（本机发起、
> 携带"出发地/当前远端"上下文、会话令牌隔离、Windows 仅作本机侧）保持不变。

## 1. 背景

用户经常从本机通过 SSH 登录远端机器，并在远端工作过程中临时传输文件或目录。
虽然 `~/.ssh/config` 已能省略用户名、地址、端口和密钥，`scp` / `rsync` 仍要求用户：

1. 离开当前远端操作上下文或另开一个本机终端；
2. 回忆远端主机别名和当前远端路径；
3. 手工拼接传输方向、源路径和目标路径。

本功能不是为 `scp` / `rsync` 更换命令名，而是让通过 shine 建立的 SSH 会话天然携带
“本机从哪里出发、远端当前在哪里”的上下文，使用户可以直接在远端发起双向传输。

## 2. 产品目标

- 用户通过 `shine ssh <host>` 登录后，可在远端当前 shell 中传输文件和目录。
- 不再重复填写主机名，也不需要另开本机终端。
- 未指定本机目标时，以启动 `shine ssh` 时的本机工作目录作为默认落点。
- 远端相对路径始终相对执行命令时的远端工作目录解析。
- 复用 OpenSSH 与 `~/.ssh/config` 的认证、跳板机和主机校验能力。
- 传输通道仅服务于当前 SSH 会话，断开后失效。

## 3. 非目标

- 不替代通用的 `scp`、`sftp` 或 `rsync`。
- MVP 不提供目录增量同步、镜像删除或双向同步。
- MVP 不提供断点续传。
- MVP 不支持从普通 `ssh <host>` 启动的既有会话自动接入。
- 不在远端永久运行守护进程，也不开放可被其他主机访问的监听端口。

## 4. 核心概念

### 4.1 会话本机目录

执行 `shine ssh <host>` 时，本机的当前工作目录被记录为该 SSH 会话的
`local session directory`。远端向本机发送内容且未显式指定本机目标时，内容写入该目录。

进入远端后，无论远端执行多少次 `cd`，该本机目录保持不变。

### 4.2 路径归属

命令参数按其所属机器解析：

- 远端路径由远端 shine 解析；相对路径基于远端当前工作目录，`~` 指远端 HOME。
- 本机路径由本机 shine 解析；相对路径基于会话本机目录，`~` 指本机 HOME。
- shell 在调用 shine 前可能展开未加引号的 `~`。文档示例应优先使用绝对路径、相对路径，
  或对需要交给另一端解析的路径加引号。

### 4.3 传输方向

命令均从远端视角命名：

- `shine local download`：将远端内容下载到本机。
- `shine local upload`：将本机内容上传到远端。

选择 `download` / `upload` 而非 `get` / `put`，避免用户需要判断动词是从本机还是远端视角
命名。

## 5. 用户流程

### 5.1 建立会话

```bash
cd ~/biulight/a/b
shine ssh dev
```

`shine ssh` 复用 `ssh` 的参数与 `~/.ssh/config`，建立交互式 SSH 会话及会话专属传输通道。
登录成功后，远端环境中应可识别当前 shine 传输会话。

### 5.2 从远端下载到本机

```bash
cd ~/xx/c/d
shine local download result.log
```

等价传输语义：

```bash
scp dev:~/xx/c/d/result.log ~/biulight/a/b/result.log
```

显式指定本机目标：

```bash
shine local download result.log 'subdir/'
shine local download output/ '~/Downloads/build/'
```

第二个参数始终在本机解析。引号用于防止远端 shell 提前展开 `~`。

### 5.3 从本机上传到远端

```bash
cd ~/xx/c/d
shine local upload notes.txt
```

该命令读取本机会话目录中的 `~/biulight/a/b/notes.txt`，并写入远端当前目录：
`~/xx/c/d/notes.txt`。

显式指定远端目标：

```bash
shine local upload assets/ ./public/assets/
shine local upload '~/Downloads/package.zip' ./package.zip
```

第一个参数始终在本机解析，第二个参数始终在远端解析。

## 6. 命令设计

```text
shine ssh [SSH_ARGS]... <HOST> [COMMAND]

shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION]
                     [--force] [--dry-run]

shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION]
                   [--force] [--dry-run]
```

### 6.1 默认目标规则

- 文件源 + 省略目标：写入另一端默认目录，保留源文件名。
- 目录源 + 省略目标：写入另一端默认目录下的同名目录。
- 显式目标已存在且是目录：在该目录下保留源名称。
- 显式目标不存在：按该路径创建目标文件或目录。
- 多源传输不进入 MVP，避免目标判断和错误恢复变复杂。
- 默认拒绝覆盖任何既有目标；`--force` 允许覆盖文件及合并目录。
- `--force` 不删除目标目录中源目录不存在的文件。

### 6.2 目录语义

MVP 将目录作为归档流传输，保留相对目录结构和普通文件权限。必须防御：

- `..` 路径穿越；
- 绝对路径归档条目；
- 通过符号链接逃出目标目录；
- 特殊文件、设备文件和 socket；
- 解包过程中对既有目标的意外覆盖。

符号链接的 MVP 策略为保留链接本身，但只接受相对且不会逃出传输根目录的链接；其他链接
应报错并停止传输。后续实现若发现跨平台行为不可预测，可进一步收窄为默认拒绝所有符号链接。

## 7. 交互与输出

成功输出应简洁并同时展示两端路径：

```text
Downloaded 1.8 MiB
  remote: ~/xx/c/d/result.log
  local:  ~/biulight/a/b/result.log
```

大文件或目录传输显示字节数、进度和当前吞吐；非 TTY 环境输出稳定的单行结果，不绘制动态
进度条。

以下情况必须给出可操作错误：

- 当前 shell 不属于 `shine ssh` 会话；
- 本机或远端未安装兼容版本的 shine；
- 源路径不存在、不可读或类型不支持；
- 目标已存在且未传 `--force`；
- 本机 agent 已退出或 SSH 转发失效；
- 磁盘空间不足或传输中断。

## 8. 安全模型

- 传输通道只绑定 loopback 或会话专属 Unix socket，不监听公网或局域网地址。
- 每次 `shine ssh` 生成高熵、短生命周期的会话令牌；远端每个请求必须携带该令牌。
- 本机 agent 仅在当前 SSH 会话存活期间运行，SSH 退出后立即关闭。
- 本机 agent 不接受任意命令执行，只实现受限的文件读取、文件写入和能力查询协议。
- 默认禁止覆盖；目标文件先写入同目录临时文件，校验成功后原子替换或重命名。
- 日志不得记录令牌、文件内容或敏感 SSH 参数。
- 远端请求的本机路径不限制在会话目录内，因为用户可能显式上传其他本机路径；但每次路径
  必须由当前命令明确提供，不提供目录遍历或文件列表 API。

## 9. 技术方案约束

建议架构：

```text
远端 shine local
    │
    │ SSH reverse forwarding（会话专属）
    ▼
本机 shine transfer agent
    │
    └── 本机文件系统
```

- `shine ssh` 启动临时本机 agent，再调用系统 OpenSSH 客户端。
- SSH 参数应尽量原样透传，不能自行解析并重建用户的 SSH 配置语义。
- 文件内容采用流式协议；目录可采用 tar 流，但不得把完整内容读入内存。
- 协议必须包含版本协商、内容长度或流结束标记、错误帧和完整性校验。
- SSH 已提供加密和服务器身份校验，不额外发明传输加密；会话令牌用于隔离同机其他进程或
  会话。

### 9.1 实现前技术尖刺

正式开发前必须验证并记录以下结果：

1. OpenSSH 在 macOS、Linux 和 Windows 上分配、发现并传递随机反向端口的可靠方式；优先
   评估 Unix socket 转发，Windows 使用 loopback TCP 回退。
2. 如何在不破坏登录 shell、TTY、远端命令和用户 SSH 参数的情况下，把会话端点与令牌传给
   远端 shine。
3. ProxyJump、多层 SSH、ControlMaster 和多个并发 `shine ssh` 会话的隔离行为。
4. SSH 断开、agent 崩溃和用户按 Ctrl-C 时，两端进程与临时文件能否可靠清理。
5. tar 流在 Unix/Windows 间的权限、符号链接和路径编码兼容性。

如果随机反向端口无法稳定发现，MVP 不应使用固定端口碰撞方案；应改用会话专属 socket，或
先建立可查询的 SSH control connection 后再申请转发。

## 10. 兼容性与版本

- 本机与远端 shine 必须完成协议版本协商。
- 主版本不兼容时拒绝传输并提示升级哪一端。
- `shine ssh` 可正常连接未安装 shine 的远端，但 `shine local ...` 不可用；SSH 本身不得因此
  失败。
- MVP 目标平台：macOS 和 Linux 互传；Windows 支持取决于技术尖刺结果，不以牺牲安全边界
  为代价强行纳入首版。

## 11. 验收标准

1. 从本机目录 A 执行 `shine ssh dev`，远端切换到目录 B 后执行
   `shine local download file`，文件准确落到本机 A，内容一致。
2. 远端目录 B 执行 `shine local upload file`，读取本机 A 中的文件并写入远端 B，内容一致。
3. 文件和嵌套目录均可双向传输，空目录得到保留。
4. 文件名包含空格、非 ASCII 字符时传输成功。
5. 目标已存在时默认拒绝，`--force` 后行为符合默认目标规则。
6. 两个并发 `shine ssh` 会话不会串用 agent、令牌或本机目录。
7. SSH 断开后，agent、转发和临时文件均被清理。
8. 路径穿越、绝对归档条目和越界符号链接测试均被拒绝。
9. 传输 1 GiB 文件时内存占用保持有界，不随文件大小线性增长。
10. 普通 `ssh` 功能、`~/.ssh/config`、ProxyJump 和主机密钥校验不发生回归。

## 12. 后续候选能力

- 断点续传与内容校验重试；
- 基于 rsync 算法的目录增量同步；
- 多文件源与 include/exclude 规则；
- 本机文件选择器；
- 经用户显式配置后，让普通 SSH 会话接入本机 agent；
- `shine local status` 显示会话本机目录、连接状态和协议版本。
