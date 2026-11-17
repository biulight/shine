---
title: Claude Code launcher
sidebar_position: 5
---

# Claude Code launcher

The `agent` preset provides `ccenv` to launch Claude Code with a selected provider:

- Store credentials encrypted with Shine, then decrypt and pass them to Claude Code at launch, avoiding long-term plaintext keys in shell startup files or provider configuration.
- Switch the provider's service address and model settings together.
- Apply the configuration to the new Claude Code process without changing the current terminal environment.

## Prepare an encrypted credential

Install Bun, Shine, and Claude Code. Configure a [GPG](../guides/environment.md#encrypt-values-with-gpg)
or [age](../guides/environment.md#use-age-identities) backend before encrypting credentials.
The bundled choices use these keys:

| Provider | Credential | Encrypted Shine variable |
| --- | --- | --- |
| Codex through local CLIProxyAPI (default) | CLIProxyAPI access token | `CLIPROXYAPI_AUTH_TOKEN_SECRET` |
| DeepSeek | DeepSeek API Key | `DEEPSEEK_API_KEY_SECRET` |
| Qwen | Qwen API Key | `QWEN_API_KEY_SECRET` |

For example, if `DEEPSEEK_API_KEY` is already stored as plaintext in Shine, encrypt it with:

```bash
shine env secret encrypt --from DEEPSEEK_API_KEY --set DEEPSEEK_API_KEY_SECRET
```

Encryption leaves the original plaintext entry in place. After confirming an encrypted launch below,
remove that entry from its [configuration layer](../guides/environment.md#inspect-and-set-values),
along with any plaintext copies in your shell or application settings. Do not put credentials in
the preset scripts or provider configuration.

For the Codex choice, CLIProxyAPI must be running at `http://127.0.0.1:8317` with the same access
token configured on both sides. Keep it bound to loopback. `ccenv` manages the token used to access
that service; it does not manage CLIProxyAPI's own upstream credentials.

## Install and launch

```bash
shine shell install agent
```

Open a new shell and run:

```bash
ccenv
```

Choose the provider whose credential you prepared; pressing Enter selects Codex. `ccenv` decrypts
the selected credential and starts Claude Code without a separate `decrypt` or `export` command.
Complete any PIN, touch, or other authorization requested by your configured decryption backend.

Claude arguments are forwarded unchanged. For a one-shot prompt, for example:

```bash
ccenv --print "hello"
```

The provider prompt still appears. Leading `-r` and `--run` are launcher compatibility options;
to pass one through to Claude instead, insert `--` first:

```bash
ccenv -- --run
```

## Customize providers and models {#customize-providers}

Use an [overlay](../guides/custom-presets.md#override-selected-files-with-an-overlay) to override
`shell/agent/cc.providers.toml`. This complete example keeps only DeepSeek; built-in providers not
listed in the file are not retained.

```toml
defaultProvider = "deepseek" # Default selection; must match an enabled provider's ID

[[providers]]
id = "deepseek" # Start with a-z; use only a-z, 0-9, _ and -
label = "DeepSeek" # Display name in the menu
enabled = true
credentialKey = "DEEPSEEK_API_KEY" # Shine variable name, not the key value

[providers.env] # All values must be strings
ANTHROPIC_BASE_URL = "https://api.deepseek.com/anthropic" # Service address
ANTHROPIC_MODEL = "deepseek-v4-pro[1m]" # Default model
# Models used for Claude Code's Opus, Sonnet, and Haiku choices
ANTHROPIC_DEFAULT_OPUS_MODEL = "deepseek-v4-pro[1m]"
ANTHROPIC_DEFAULT_SONNET_MODEL = "deepseek-v4-pro[1m]"
ANTHROPIC_DEFAULT_HAIKU_MODEL = "deepseek-flash"
CLAUDE_CODE_SUBAGENT_MODEL = "deepseek-flash" # Default subagent model
```

Store the encrypted credential in `DEEPSEEK_API_KEY_SECRET` as described above; do not put the key
value in this file. To add another provider, add a `[[providers]]` entry with its own `[providers.env]`
and a unique `id`.

After linking the overlay, install as above for first use. For an existing installation in the
default snapshot mode, apply the new configuration with:

```bash
shine update shell/agent
shine upgrade shell/agent
```

## Credential behavior and limits

For a credential named `KEY`, resolution prefers `KEY_SECRET`, then legacy `KEY_GPG_SECRET`, then
plaintext `KEY` if neither encrypted entry exists. A decryption failure stops the launch without
falling back. Using `ccenv` alone does not encrypt a plaintext credential.

Encrypted storage reduces persistent plaintext copies. During use, the launcher and Claude Code
still receive plaintext; Claude Code, its child processes, and processes with sufficient permissions
may read it. Encryption does not isolate credentials from the program using them.
