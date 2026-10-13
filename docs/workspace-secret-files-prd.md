# Workspace 机密文件组 PRD

**状态：软件实现完成，Linux／macOS／Windows 原生自动回归通过；硬件及真实 Android 签名待验收，尚未发布。** 日期：2026-10-12；验证更新：2026-10-13。

本文为内部需求，不发布到公共网站。设计记录见
[ADR 0110](kb/decisions/0110-workspace-secret-file-groups.md)。

## 1. 背景与目标

项目构建依赖签名私钥、keystore、密码文件及服务凭据。现有 workspace 管理环境变量，
无法声明机密文件映射和明文生命周期。Lumeport 的 Android 签名是需求来源，验证不得读取
或操作其真实签名资料。

新增命名文件组，在同一份映射中支持两个方向：

```text
显式封存：target 明文 → 加密 → source 仓库密文
临时执行：source 仓库密文 → 解密 → target 本地明文 → 子命令 → 清理
```

成功标准：仓库只需提交密文和映射；解密身份留在仓库外；多个组可覆盖多个目录；
不同环境选择对应路径；任意字节无损还原；全部文件准备成功后才启动命令；正常结束、
失败和受控中断清理本次文件。应用代码和 Gradle 只读取普通文件，自动解密构建入口依赖 Shine CLI。

## 2. 配置和兼容性

```toml
version = 3

[env]
default_mode = "staging"
modes = ["staging", "production"]

[env.encryption]
backend = "age"
age_recipients = ["<实际 age recipient>"]

[[file_groups.android-signing.files]]
source = ".secrets/{mode}/release.jks.enc"
target = ".android-signing/release.jks"

[[file_groups.android-signing.files]]
source = ".secrets/{mode}/signing.properties.enc"
target = ".android-signing/signing.properties"

[[file_groups.service-account.files]]
source = ".secrets/{mode}/service-account.json.enc"
target = "app/.credentials/service-account.json"
```

- 文件组是命名映射集合，不是目录；同组可跨目录，多组可组合。组名仅含字母、数字、
  短横线和下划线，组内 `files` 必须非空，每项要求 `source` 和 `target`。
- 两个路径均相对 workspace 所在目录解析，支持 `{mode}`。命令行 mode 优先于默认值；
  两者都没有时失败。mode 必须属于 `env.modes` 与 `env.default_mode` 的并集。
- 所有环境使用相同映射清单。文件缺失时失败，不跨环境回退。
- 新增 version 3，现有 version 2 行为保持。文件组要求 version 3，使旧客户端明确拒绝。
- Version 3 有文件组时可省略 `env.files`；纯文件 workspace 的 `env run` 必须选择文件组。
- 复用现有 CLI → workspace → 全局加密配置优先级及 hybrid 收件人限制，不新增组级或环境级策略。

## 3. 命令接口与流程

```bash
shine env secret seal --mode production --file-group android-signing --file-group service-account
shine env secret seal --mode production --file-group android-signing --file-group service-account --remove-plaintext
shine env run --mode production --file-group android-signing --file-group service-account -- pnpm android:build
```

`--file-group` 可重复，重复组去重，未知组报错。不根据命令名称自动选组。
不带参数的普通封存和运行不隐式处理文件组。文件组封存与位置参数 `seal <FILE>` 互斥；
文件组运行与 `--no-workspace`、`--secret-broker` 互斥。

### 封存

先统一预检并捕获配置、明文和已有密文，再完成全部加密；确认输入未变化后，
逐文件原子创建或更新密文。默认保留明文，不要求可用私钥。已有 source 必须通过密文格式检查。
多文件写入不是整体事务；失败报告已完成项和未确认项，保留明文。

`--remove-plaintext` 在全部密文写入成功后重新读取并解密，逐字节对照捕获明文；
再次复核明文、密文和配置，全部通过后才开始删除。授权取消或验证失败时不开始删除。
删除阶段失败即停止，报告已删除和保留文件，不承诺整体删除事务。

### 临时执行

先捕获、检查并解密所有选中文件，再以禁止覆盖和私有权限创建目标。所有目标就绪后启动
子命令，保留现有环境注入能力。命令结束后按文件身份清理，只删除本次创建且未被替换的文件，
仅移除本次创建且仍为空的目录。

准备或启动失败同样清理。受控中断先终止并回收受管进程，再清理。子命令失败保留退出码；
子命令成功但清理失败返回非零。清理必须发生在直接退出之前。文件被替换时保留并报告。
生命周期以包装命令为界，不支持命令结束后仍依赖这些文件的后台任务。

### 预览

两个入口支持 `--dry-run`，展示环境、组、映射及元数据冲突，不读取明文内容、调用密码工具、
创建文件或启动子命令。不验证密文完整性和私钥可用性。可以预览带 `--remove-plaintext` 的封存。

## 4. 安全约束

- 路径限定在 workspace 内；拒绝绝对路径、父目录穿越、符号链接、Windows 重解析点、
  非普通文件和硬链接。保守拒绝大小写别名及路径祖先冲突，source 与 target 不得重叠。
- 运行要求全部 target 不存在，无强制覆盖。Git 仓库中的明文目标必须未跟踪且被忽略，
  不自动修改 `.gitignore`。Git 元数据检查不得启动 fsmonitor 等仓库指定程序。
- Unix 新文件从创建起为 `0600`、新目录为 `0700`。Windows 从创建起使用当前用户私有 ACL，
  文件系统无法提供私有 ACL 时在写出明文前失败。
- 相关路径共用持久 OS 锁，冲突立即失败；绑定捕获输入，写入和删除前复核。
  不合作进程在最终检查到系统调用之间的竞态不属于可移植锁保证。
- 使用现有 GPG／age／hybrid 密文格式和自描述路由，新增字节 API，保留环境变量字符串 API。
  支持空文件、NUL、非 UTF-8，不转换换行。每文件 16 MiB，单次明文总量 64 MiB，编码密文单文件 32 MiB、单次 128 MiB，实行有界读取。
- Hybrid 每次操作固定本机选定后端，失败或取消不切换后端，不缩减收件人。
- 不持久化明文缓存或数据密钥；状态和错误不得包含明文、密码、完整密文或解密解析片段。
- 子命令及有权限的本机进程可以读取明文，本功能不是沙箱。构建结束只清理，不自动回写密文。
- 强制杀进程或断电可能残留明文；后续发现已有目标时拒绝执行并要求人工检查。
  不自动认领残留，删除不代表安全擦除，删除收件人无法撤销历史密文访问。

## 5. 首版非目标

不提供持久还原、独立残留清理、模板、递归目录、glob、归档、权限自定义、条件映射、组继承、
环境级接收者、构建监听、透明代理、自动回写、SSH 文件释放、远程签名、KMS、身份安装、
仅从密文更新收件人的专用流程、历史密文撤销或真实凭据轮换。

## 6. 验收

| 场景 | 通过条件 |
|---|---|
| 文本、二进制、空文件 | 三种后端逐字节往返一致 |
| 多目录、多组、不同 mode | 全部就绪后启动；缺失不回退 |
| 纯文件 workspace、旧版配置 | 新功能无需占位源；version 2 不回归 |
| 未知组、链接、路径重叠或越界 | 在执行命令或破坏性操作前拒绝 |
| 已有目标、被替换目标 | 不覆盖、不误删，并明确诊断 |
| Git 跟踪／忽略状态 | 非忽略或已跟踪的明文被拒绝 |
| 加密失败、输入变化、删除前取消或验证失败 | 不写出无效密文，不删除原始明文 |
| 构建成功、失败、启动失败和中断 | 清理本次文件，正确返回状态 |
| 并发或清理失败 | 明确冲突或失败，不互相覆盖 |
| 预览 | 不读明文、不调用密码工具、不写文件、不启动命令 |
| 旧客户端 | 拒绝 version 3 |

测试只使用临时 workspace、隔离 `SHINE_CONFIG_DIR`、测试后端或一次性真实密钥。
真实 Android 签名由用户运行新入口后单独记录，不使用实际私钥进行自动验收。

## 7. 文档和发布

同步维护中英文手册、CLI 帮助和补全、手写 CHANGELOG、ADR 和 KB 不变量及数据流。
本 PRD 不进入公共网站。平台、硬件和真实 Android 签名只按实际验证结果声明；
软件测试不替代 Touch ID、手机授权、Linux／Windows 原生文件权限和进程行为验收。

## 8. 实现与验证记录

- 实现 version 3 文件组、双向映射、二进制密码 API、私有创建、路径锁、身份清理、
  受控取消及 Unix 进程组／Windows Job Object 管理。密文写入在删除明文前完成持久化同步。
- macOS 全量 nextest：1499 项通过，2 项按原配置跳过；最终持久化调整后，12 项文件组 CLI 回归复测通过。
- 一次性真实 GPG／age／hybrid 互操作测试通过，包含空文件和非 UTF-8 数据。
- Rust Clippy、格式、依赖策略、typos、中英内容一致性、网站类型检查与双语生产构建通过。
- Windows 原生平台模块通过独立交叉编译与 Clippy。完整 Windows 交叉构建受本机缺少 Windows C SDK 阻断；
  独立模块检查不代表整个程序或 Windows 运行时验收。
- 2026-10-13：修复 Windows 同名导入、重复封存的旧密文句柄冲突和测试就绪文件竞态，
  并将 quinn-proto 升级至 0.11.18。`5da8168` 的 Linux／macOS／Windows 原生测试、质量与安全审计
  及 Rust 1.88 检查均通过（[CI](https://github.com/biulight/shine/actions/runs/38350246715)）。
- 沙箱内 sccache、测试 gpg-agent 与 Unix socket 受权限限制；清空编译包装器，并通过自动审批在沙箱外
  运行隔离密码互操作和全量回归后通过。依赖策略检查保留现有重复版本和未使用许可证许可警告。
- 未操作实际项目签名密钥，未执行真实 Android 签名；原生平台结论限于自动回归覆盖，硬件授权仍未验证。
