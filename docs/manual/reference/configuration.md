---
title: Configuration reference
sidebar_position: 2
---

# Configuration reference

Shine stores global runtime state under `~/.shine/` and creates `~/.shine/config.toml` when global
configuration is first needed.

## Common global fields

```toml
presets_dir = "~/dotfiles/shine-presets"
# External shell presets default to snapshot; use live only for preset development.
external_shell_mode = "live"
presets_overlay_git = "https://example.com/team/shine-overlay.git"
presets_overlay_git_branch = "main"
app_default_dest_root = "~/.config"
allow_app_hooks = true
allow_sys_code = true
sync_terminal_theme = true
gpg_recipients = ["user@example.com", "team-backup@example.com"]

secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"

[env]
HTTP_PROXY_PORT = "6152"
SOCKS5_PROXY_PORT = "6153"
PROXY_HOST = "127.0.0.1"
PROXY_NO_PROXY = "localhost,127.0.0.1,::1"
MY_API_TOKEN = { value = "<token>", description = "Token for the internal API" }

[[env_proxy]]
command = "gh"
with = ["GH_TOKEN"]
# Defaults to true; false forwards without decrypting or injecting values.
enabled = false
```

| Field | Purpose |
| --- | --- |
| `presets_dir` | Replace built-in presets with a complete external preset directory |
| `external_shell_mode` | External shell deployment mode; `snapshot` by default, optionally `live` |
| `presets_overlay_git` | Git overlay URL shallow-cloned and mirrored under `~/.shine/overlay/` |
| `presets_overlay_git_branch` | Tracked overlay branch; omit for the remote default branch |
| `app_default_dest_root` | Default root for legacy application presets without a destination |
| `allow_app_hooks` | Permit lifecycle hooks in external application presets |
| `allow_sys_code` | Global-only permission for external sys scripts and persistent executable profile code; project config cannot enable it |
| `sync_terminal_theme` | Enable automatic terminal theme synchronization in managed Unix profiles; defaults on |
| `gpg_recipients` | Default GPG recipients for `shine env secret encrypt` |
| `secret_backend` | Default secret backend; `gpg` when omitted |
| `age_recipients` | Default encryption recipients for age |
| `age_identity` | Identity file for `age:` ciphertext; may default to `~/.shine/age/identity.txt` |
| `[env]` | Values used by templates and shell helpers |
| `[[env_proxy]]` | Transparent command rule: bare `command`, injected `KEY` or `KEY=ALIAS` list in `with`, and optional `enabled` defaulting to `true` |

Legacy `gpg_key_id` and workspace `[env.encryption].recipient` accept only one recipient. Normal
configuration reads never rewrite them. Preview and apply conversion to `gpg_recipients` with
`shine state migrate --dry-run` and `shine state migrate`. `env run` and `env secret seal` prompt to
migrate an old workspace when needed.

## Environment entry formats and descriptions

`[env]` in global and project configuration, plus every `shine.env.toml` override, accepts a string or
a detailed value:

```toml
[env]
PLAIN_VALUE = "example"
DETAILED_VALUE = { value = "example", description = "Example value used by build tasks" }
```

- A string suits a value that needs no explanation.
- Detailed `value` behaves like a string in `env get`, template replacement, secret encrypt/export,
  and `env run --with`.
- `description` appears only in `shine env list` and is never passed to templates or children.
- Inline descriptions override the same key in the preset `<presets>/env.toml` catalog.
- `shine env set` preserves the description of an existing detailed entry.
- A detailed override replaces both value and description; a string replaces only the value.
- Invalid numbers, arrays, or tables without a string `value` report file and key instead of being
  ignored.

## Project configuration

Shine searches upward from the current directory for the nearest `shine.config.toml`. It is a sparse
layer over global configuration: omitted fields inherit, and relative paths resolve from the file
that declares them.

`[[env_proxy]]` follows the same layering. A project rule replaces a global rule with the same
`command`; unrelated global rules remain. Prefer `shine env proxy install`, `enable`, and `disable` so
configuration remains consistent with the shim under `~/.shine/bin/`.

Shine 0.40.0 stopped recognizing project `config.toml` and `.env.toml`; rename them to
`shine.config.toml` and `shine.env.toml`. Ordinary files with the old names are ignored.

It also stopped automatically migrating global `~/.shine/env.toml`. Before upgrading, load it once
with v0.39. After upgrading, move it to `~/.shine/shine.env.toml`, manually merging an existing
destination. Detection of the old file stops normal configuration loading with recovery instructions
instead of silently dropping active values.

## Directory and source precedence

`SHINE_CONFIG_DIR` has the highest priority. It changes the global configuration and runtime root
and, outside a project, fixes the preset directory at `$SHINE_CONFIG_DIR/presets/`; `SHINE_PRESETS`
and a global `presets_dir` do not override it.

When a project `shine.config.toml` is active, `SHINE_CONFIG_DIR` still selects the runtime root, but
an explicit `SHINE_PRESETS` or project `presets_dir` can select the preset source. An inherited
global `presets_dir` cannot override `$SHINE_CONFIG_DIR/presets/`.

Without `SHINE_CONFIG_DIR`, the base preset directory is selected in this order:

1. `SHINE_PRESETS`;
2. project `shine.config.toml` `presets_dir`;
3. global `config.toml` `presets_dir`;
4. default `~/.shine/presets/`.

External shell categories default to `snapshot`: Shine copies them to
`~/.shine/installed/shell/`. After source edits, inspect with `shine update` and apply with
`shine upgrade`. Set `external_shell_mode = "live"` only for development; source content takes effect
on the next invocation, but changes to `target`, `runtime`, `transforms`, or `env` still require
`shine upgrade` to rebuild managed entries.

An overlay replaces files at matching relative paths over the chosen base instead of replacing the
whole tree. `presets_overlay_dir` and `presets_overlay_git` are mutually exclusive; use
`shine preset overlay link` to configure them safely. A Git overlay becomes active after the first
successful `shine preset pull`. Later pulls forcibly mirror its checkout, so never edit
`~/.shine/overlay/` directly.

## Environment value precedence

Later layers override earlier values:

1. built-in defaults;
2. global `[env]`;
3. project `[env]`;
4. global `~/.shine/shine.env.toml`;
5. active overlay `shine.env.toml`;
6. project `shine.env.toml`.

An override file has no `[env]` header:

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "Local proxy host" }
```

## Workspace environments

`shine.workspace.toml` declares modes, ordered environment sources, and shared encryption
recipients:

```toml
version = 2

[env]
modes = ["development", "production"]
default_mode = "development"
override_process_env = false
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
gpg_recipients = ["user@example.com", "team-backup@example.com"]
# Or use age:
# backend = "age"
# age_recipients = ["age1se1qexample...", "age1qteammate..."]
```

Sources merge in list order. Current process variables win by default;
`env.override_process_env = true` lets workspace values replace them.

For `shine env secret seal` and `shine env run`, encryption settings resolve from command-line
arguments, then `[env.encryption]`, then global `~/.shine/config.toml`. Global configuration is useful
for personal defaults; commit project-team GPG or age recipient lists here. Recipients are public-key
information. Never commit private identity files such as `age_identity`.

Each source uses this structure:

```toml
version = 1

[plain]
PUBLIC_VALUE = "example"

[secret]
EXISTING_SECRET = true
PROMPT_ON_SEAL = false
PLAINTEXT_TO_SEAL = "<value to seal>"

[payload]
data = "<GPG ciphertext managed by Shine>"
```

`shine env secret seal` merges pending values into the encrypted payload and changes sealed entries
to `true`. `shine env run` merges `[plain]` and decrypted values in source order. With usable GPG
recipients it also maintains an encrypted, mode-specific cache.

`shine env run --with KEY[=ALIAS]` injects a value from current Shine `[env]`, preferring
`KEY_SECRET` and then `KEY`. Explicit injection overrides workspace and process values.

## Managed directories

```text
~/.shine/
├── config.toml
├── shine.env.toml
├── app-manifest.toml
├── shell-manifest.toml
├── proxy-manifest.toml
├── tasks.toml
├── bin/
├── http/
├── installed/
├── overlay/
├── rendered/
└── presets/
    ├── app/
    ├── shell/
    └── sys/
```

Do not delete manifests manually and expect Shine to rediscover old installations. Prefer the
corresponding `uninstall --dry-run` and `uninstall` commands.
