---
title: Claude Code 启动器
sidebar_position: 5
---

# Claude Code 启动器

`agent` 预设提供 `ccenv`，按所选 provider 启动 Claude Code：

- 使用 Shine 加密保存凭据，启动时解密并传入 Claude Code，避免将密钥长期明文写入 Shell 启动文件或 provider 配置。
- 一并切换 provider 的服务地址和模型设置。
- 配置作用于新启动的 Claude Code，不改写当前终端环境。

## 准备加密凭据

先安装 Bun、Shine 和 Claude Code，并按 [GPG](../guides/environment.md#使用-gpg-加密值) 或
[age](../guides/environment.md#使用-age-identity) 指南配置加密后端。内置选项使用以下变量：

| Provider | 凭据用途 | Shine 中的加密变量 |
| --- | --- | --- |
| 通过本机 CLIProxyAPI 使用 Codex（默认） | CLIProxyAPI 访问令牌 | `CLIPROXYAPI_AUTH_TOKEN_SECRET` |
| DeepSeek | DeepSeek API Key | `DEEPSEEK_API_KEY_SECRET` |
| Qwen | Qwen API Key | `QWEN_API_KEY_SECRET` |

例如，若 Shine 中已经以明文保存了 `DEEPSEEK_API_KEY`，可将其加密：

```bash
shine env secret encrypt --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_SECRET
```

加密不会删除原明文条目。确认下文的加密凭据启动成功后，再从对应的
[配置层](../guides/environment.md#查看和设置变量)删除明文条目，并清理 Shell 或应用设置中的明文副本。
不要把凭据写进预设脚本或 provider 配置。

选择 Codex 时，CLIProxyAPI 需已在 `http://127.0.0.1:8317` 运行，客户端与服务端使用相同令牌，
并保持只监听回环地址。`ccenv` 管理的是访问该服务的令牌，不管理 CLIProxyAPI 自身的上游凭据。

## 安装与启动

```bash
shine shell install agent
```

打开新的 Shell，运行：

```bash
ccenv
```

选择已准备凭据的 provider，直接回车默认选择 Codex。`ccenv` 会解密所选凭据并启动 Claude Code，
无需单独运行 `decrypt` 或 `export`。按配置的解密后端要求完成 PIN、触摸或其他授权。

Claude 参数会原样转发。例如，执行单次提示：

```bash
ccenv --print "hello"
```

此时仍会出现 provider 选择提示。开头的 `-r`、`--run` 会被当作启动器兼容选项；如需原样传给
Claude，先写 `--`：

```bash
ccenv -- --run
```

## 自定义 provider 与模型 {#customize-providers}

通过 [overlay](../guides/custom-presets.md#使用-overlay-覆盖少量文件) 覆盖
`shell/agent/cc.providers.toml`。以下完整示例只保留 DeepSeek；未列入文件的内置 provider 不会保留。

```toml
defaultProvider = "deepseek" # 默认选项，必须对应已启用 provider 的 ID

[[providers]]
id = "deepseek" # 以 a-z 开头，仅使用 a-z、0-9、_ 和 -
label = "DeepSeek" # 菜单中的显示名称
enabled = true
credentialKey = "DEEPSEEK_API_KEY" # Shine 变量名，不是密钥值

[providers.env] # 所有值均使用字符串
ANTHROPIC_BASE_URL = "https://api.deepseek.com/anthropic" # 服务地址
ANTHROPIC_MODEL = "deepseek-v4-pro[1m]" # 默认模型
# Claude Code 中 Opus、Sonnet、Haiku 选项对应的模型
ANTHROPIC_DEFAULT_OPUS_MODEL = "deepseek-v4-pro[1m]"
ANTHROPIC_DEFAULT_SONNET_MODEL = "deepseek-v4-pro[1m]"
ANTHROPIC_DEFAULT_HAIKU_MODEL = "deepseek-flash"
CLAUDE_CODE_SUBAGENT_MODEL = "deepseek-flash" # 子代理默认模型
```

按上文将加密凭据保存在 `DEEPSEEK_API_KEY_SECRET`，不要把密钥值写入此文件。
新增 provider 时，添加一组 `[[providers]]` 及其 `[providers.env]`，使用不同的 `id`。

关联 overlay 后，首次使用按上文安装；已有安装在默认 snapshot 模式下，通过以下命令应用新配置：

```bash
shine update shell/agent
shine upgrade shell/agent
```

## 凭据行为与限制

对于名为 `KEY` 的凭据，优先读取 `KEY_SECRET`，其次是旧版 `KEY_GPG_SECRET`；两者都不存在时才
使用明文 `KEY`。所选密文解密失败即停止启动，不会回退。仅使用 `ccenv` 不会自动加密明文凭据。

加密存储减少的是持久保存的明文副本。使用时，启动器和 Claude Code 仍会拿到明文，Claude Code、
其子进程以及有足够权限的进程可能读取它；加密不构成对用钥程序的隔离。
