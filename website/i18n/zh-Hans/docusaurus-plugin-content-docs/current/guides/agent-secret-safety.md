---
title: 在 AI Agent 参与开发时保护环境密钥
sidebar_position: 7
---

# 在 AI Agent 参与开发时保护环境密钥

Claude Code、Codex 等 AI Agent 参与开发后，密钥安全不再只是“不要提交 `.env`”这么简单。Agent 可能能读取工作区文件、运行命令、查看命令输出；如果把长期有效的明文 secret 放在项目里，它们很容易被复制到日志、补丁、上下文或远端服务中。

Shine 的 `env secret seal`、`env run` 和 `age` 后端用于降低这种扩散风险：把仓库中的 secret 保存为密文，只在需要运行命令时解密并注入子进程。但它们不是沙箱，也不能替代系统权限隔离。使用前应先明确密钥身份文件、硬件授权和 Agent 权限之间的边界。

## Shine env 保护什么

`shine env secret seal` 把 workspace 环境文件中的待处理 secret 封存到加密 payload 中。封存后，团队仓库里保留的是密文，不再是明文 token、密码或 API key。

```bash
shine env secret seal
```

`shine env run` 在启动目标命令前合并环境文件、解密 secret，并只把结果提供给这个子进程：

```bash
shine env run --mode development -- bun run build
```

这种方式主要减少三类风险：

- 明文 secret 长期留在项目文件中。
- 开发者为了运行任务，把 secret 导出到整个 shell 会话。
- AI Agent 修改代码时顺手读到、复制或提交 `.env` 明文。

但只要某个 Agent 被允许运行会读取环境变量的命令，它仍可能看到目标命令可见的 secret。`env run` 的安全边界是“按需注入”，不是“让不可信命令无法读取变量”。

偶尔执行一次需要凭据的操作时，建议由用户在可信终端通过单次 `env run --with` 注入。若某个
CLI 每次调用都需要同一个固定凭据变量，则可以安装透明命令代理，继续使用原来的调用方式：

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

如果 Agent 获准运行 `gh`，它仍然可以使用注入的 token，也能查看命令暴露的信息。透明代理
减少的是持久明文和整个 Shell 会话范围的导出，并不会让目标命令变成可信程序。设置方式、
启停语义和 Cargo 示例见[选择单次注入还是透明代理](./environment.md#选择单次注入还是透明代理)。

## age identity 是解密能力

使用 `age` 后端时，`age_recipients = ["age1..."]` 表示密文要加密给谁。recipient 类似公钥地址：个人默认值可写入 `~/.shine/config.toml`，项目团队共享的名单应写入 `shine.workspace.toml` 的 `[env.encryption]`，后者可以提交到仓库。

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
```

`~/.shine/age/identity.txt` 则是解密 identity，等同于私钥身份，不能提交、不能共享，也不应放进 Agent 可随意读取的工作区。

```bash
shine env secret identity init
shine env secret identity list
```

Shine 在 Unix/macOS 上会把自己生成的 identity 文件权限设置为 `0600`，也就是仅当前用户可读写。这可以避免其他本机用户直接读取 identity 文件。

这个权限限制仍然挡不住已经以当前用户身份运行、并被授权读取该路径的 Agent、脚本或进程。普通 age identity 保护的是仓库和传输中的密文，不是完整保护本机运行环境。

## Touch ID 改善什么

macOS 上可以生成 Secure Enclave / Touch ID identity：

```bash
shine env secret identity init --touch-id
```

这种身份由 `age-plugin-se` 生成。解密时需要本机 Secure Enclave，并触发 Touch ID 或系统 PIN 授权。即使 identity 文件被复制到另一台机器，通常也不能直接解密。

它带来的主要改进是：

- identity 不容易被复制后离线滥用。
- 解密需要用户在本机授权。
- Agent 即使能读到 identity 文件，也不能仅靠文件在别处解密。

它不是绝对隔离。若 Agent 能在本机运行解密命令，仍可能触发系统授权提示。看到意外的 Touch ID/PIN 提示时，应取消授权并检查刚才运行的命令。

## Windows 成员如何协作

Windows 成员可以使用普通 age identity 参与多 recipient 协作：

```bash
shine env secret identity init
shine env secret identity list
```

把输出中的 `age1...` recipient 加入 `age_recipients` 后，同一份密文可以同时加密给 macOS Touch ID recipient 和 Windows 普通 age recipient。

如果团队希望每次解密都需要一次新的硬件授权，可以了解由 Shine 作者另行开发的独立项目 [`age-plugin-phone`](https://github.com/biulight/age-plugin-phone)。它源自 Shine 对 Windows 硬件 identity 的直接探索：这项工作在实现过程中碰到了平台能力上限。Shine 的 PoC 发现，Windows Hello 的 Passport provider 只能完成旧式 RSA PKCS#1 v1.5 解包；RSA OAEP-SHA256、P-256 ECDH 和测试过的 WebAuthn PRF 路径都不可用。因此，作者没有把旧式构造或 Shine 专用密文格式放进产品，而是把后续实现移到标准 age plugin 协议之后，并拆分为一个可以独立审查的项目。

当前实现把长期 age 解密密钥留在 Android StrongBox 中，每次解包 file key 都需要用户在手机上重新完成一次强生物验证。Windows TPM 只保存两把用途分离、不可导出的 P-256 密钥，分别用于证明已配对桌面身份和私下选择对应的 recipient stanza；手机的长期私钥不会进入 Windows，也不存在 DPAPI、软件 identity、密码或缓存授权回退。Shine 仍然只调用标准 `age` CLI、`identity-v1` 和 `recipient-v1`，不依赖这个项目，也不引入专用密文格式。

这个设计绕开的是 Windows Hello 缺少所需密码学操作的瓶颈，并没有消除当前的平台前置条件。当前仍是 owner-only 技术预览，要求 Windows 11 x64 客户端、TPM 2.0、Microsoft Platform Crypto Provider，以及能力检查合格的 Android StrongBox 手机。它的 `auto` 策略现在会在配对或解包前先做一次有界的 Wi-Fi-first 路径选择：只有一个匹配且位于前台的 listener 响应时选择 Wi-Fi；没有 listener 响应时，会在协议处理开始前选择 Windows 的 Developer USB/ADB。多个响应会安全失败，请求发出后不会再自动切换；QR 仍是显式路径。协议 v2、公共签名、多设备覆盖和完整生命周期矩阵都还没有完成。它只能用于合成或可丢弃数据，不能保护真实或生产 secret。具体的制品校验、配对、传输、恢复和清理步骤以项目的 [`Windows Alpha quick start`](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-alpha-quickstart.md) 为准。

这个 plugin 只使用 Shine 现有的 age identity 和 recipient 配置。具体的本机与 workspace 设置见[在 Windows 上实验手机授权](./environment.md#在-windows-上实验手机授权)。identity stub 只包含公开配对材料，不含手机的长期私钥。

在当前支持的预览平台上，`shine env secret identity init --phone` 会启动 plugin 自己的事务式
setup，并且只把公开 stub 路径写入全局 `age_identities`。它不会管理 plugin 私有状态、切换默认
后端，也不会创建只包含 phone recipient 的收件人集合。

恢复路径不能依赖同一部手机的 StrongBox 密钥、同一台 Windows 电脑的 TPM 密钥或该 plugin 的本地状态。对于需要保留的数据，绝不能只配置这个实验性手机 recipient。普通团队开发仍可使用普通 age identity，但要保护好 identity 文件和用户目录权限。Windows 上需要稳定硬件保护时，应继续采用组织认可的 YubiKey/PIV 或 GPG + YubiKey 方案。

## 如何选择密钥后端

可以按下面的粗略顺序理解安全强度：

1. GPG + YubiKey / 硬件智能卡
2. age + Secure Enclave / Touch ID
3. 普通 age identity 文件
4. 明文 secret

`age + Touch ID` 通常比 `GPG + YubiKey` 更顺手，适合团队开发和预发环境。生产高价值 secret、长期凭据或需要强硬件隔离的场景，仍建议使用 GPG + YubiKey 或组织认可的硬件密钥方案。

实验性手机方案的目标也是提供硬件托管和逐次授权隔离，但在协议与发布门槛全部完成前，不把它列入上述稳定后端排序。

## 给 AI Agent 的权限建议

把 Agent 当作一个有能力的本机协作者，而不是一个天然可信的安全边界。为它准备权限时，优先遵守这些规则：

- 不把 `~/.shine/age/identity.txt`、GPG 私钥、云厂商 credential 文件加入工作区。
- 不让 Agent 长时间运行带有高权限 secret 的交互式 shell。
- 需要 secret 时，优先让 Agent 修改代码，由用户在可信终端中执行 `shine env run`。
- 对低风险开发任务使用普通 age identity；对高敏感任务使用 Touch ID 或 YubiKey；`age-plugin-phone` 仅限其文档规定的合成数据预览。
- 看到非预期的 Touch ID、手机生物验证、PIN 或 YubiKey 触摸提示时取消授权。

如果 identity 文件泄露、设备不再可信，或成员离开团队，仅从 `age_recipients` 删除 recipient 不会撤销它对历史密文的访问。需要重新封存或重新加密，并在必要时轮换上游服务中的真实 token。

```bash
shine env secret seal
```

不要提交含有尚未封存字符串的环境文件。个人覆盖文件应加入 `.gitignore`，团队共享文件中只保留已封存的密文。
