---
title: Configuration reference
sidebar_position: 2
---

# Configuration reference

Shine stores global configuration and state under `~/.shine/` and creates `config.toml` when needed.
On Unix, when `SUDO_USER` identifies the invoking user, home-relative paths use that user's home.

Use this page to look up settings and their precedence. For step-by-step use, see
[environment variables and secrets](../guides/environment.md) or
[custom presets](../guides/custom-presets.md).

## Common global fields

Add only the settings you need to `~/.shine/config.toml`, replacing example paths and recipients.
Keep secret values encrypted rather than placing them directly in `[env]`.

```toml
presets_dir = "~/dotfiles/shine-presets"
# Use live only for preset development.
external_shell_mode = "snapshot"
presets_overlay_git = "https://example.com/team/shine-overlay.git"
presets_overlay_git_branch = "main"
app_default_dest_root = "~/.config"
sync_terminal_theme = true
gpg_recipients = ["user@example.com", "team-backup@example.com"]

secret_backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]

[env]
HTTP_PROXY_PORT = "6152"
SOCKS5_PROXY_PORT = "6153"
PROXY_HOST = "127.0.0.1"
PROXY_NO_PROXY = "localhost,127.0.0.1,::1"
IMAGE_QUALITY = "80"
IMAGE_MAX_WIDTH = "1920"
IMAGE_MAX_HEIGHT = "1080"
API_URL = { value = "https://api.example.com", description = "API endpoint" }

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
| `presets_overlay_dir` | Local directory whose files override matching preset files; cannot be combined with `presets_overlay_git` |
| `presets_overlay_git` | Git overlay URL managed under `~/.shine/overlay/`; global configuration only |
| `presets_overlay_git_branch` | Tracked overlay branch; omit for the remote default branch |
| `app_default_dest_root` | Default root for legacy application presets without a destination |
| `sync_terminal_theme` | Automatic terminal theme synchronization in managed Unix profiles; defaults on and is global-only |
| `gpg_recipients` | Default GPG recipients for `shine env secret encrypt` |
| `secret_backend` | Default secret backend; `gpg` when omitted |
| `age_recipients` | Default encryption recipients for age |
| `age_identity` | Identity file for `age:` ciphertext; may default to `~/.shine/age/identity.txt` |
| `age_identities` | Additional identity-file paths, merged after `age_identity` and deduplicated |
| `[env]` | Values used by templates and shell helpers |
| `[[env_proxy]]` | Transparent command rule: bare `command`, injected `KEY` or `KEY=ALIAS` list in `with`, and optional `enabled` defaulting to `true` |

For legacy `gpg_key_id`, `[env.encryption].recipient`, or `age1se...` recipients, preview changes
with `shine state migrate --dry-run`, then run `shine state migrate` after review. Ordinary reads
do not rewrite these settings. See [encryption setup](../guides/environment.md#encrypt-values-with-gpg)
and [age identities](../guides/environment.md#use-age-identities) before resealing.
Phone recipients require a separate [manual conversion](../guides/environment.md#experiment-with-phone-authorization-on-windows).

## External-code trust

Before executing external preset code, review what it will run and access. Interactive confirmation
can authorize the listed code for one operation without saving trust. Automation and `--yes`
require an existing grant. Live Shell presets require development trust, which also authorizes
future edits. Explicit `--run-generators` checks require an existing grant too.

Use the following commands to inspect and manage trust. Do not edit `~/.shine/trust.toml` manually
or put grants in project configuration.

| Operation | Command |
| --- | --- |
| Inspect without granting trust | `shine trust inspect app/example` |
| Grant snapshot trust | `shine trust grant app/example` |
| List grants | `shine trust list` |
| Revoke trust | `shine trust revoke app/example` |
| Inspect without granting trust | `shine trust inspect shell/tools/my-command` |
| Grant snapshot trust | `shine trust grant preset` |
| Trust future changes | `shine trust grant app/example --development` |

A snapshot grant covers the target’s entire preset category and its reviewed target, source, and
capabilities; changes require review again. `--development` allows future code edits from the displayed local source, while
changes to the target, capabilities, or source directory/layer still need authorization. It does
not provide ongoing code review or replace the plan shown before each operation.

The `preset` target covers currently active targets, not future additions. Old grants may require
renewed review even if they remain listed and can be revoked. Retired `allow_app_hooks` and
`allow_sys_code` fields do not grant trust and are removed on the next configuration save.

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
- Strings and detailed entries containing only `value` inherit lower-layer descriptions. An explicit
  `description` replaces the inherited description; an empty string clears it and disables catalog
  fallback.
- Invalid numbers, arrays, or tables without a string `value` report file and key instead of being
  ignored.

## Project configuration

Shine searches upward from the current directory for the nearest `shine.config.toml`. Project
settings override global settings; omitted fields keep their global values. Relative paths start
from the directory containing the file that declares them.

`[[env_proxy]]` follows the same layering. A project rule replaces a global rule with the same
`command`; unrelated global rules remain. Prefer `shine env proxy install`, `enable`, and `disable` so
configuration remains consistent with the shim under `~/.shine/bin/`.

For old `config.toml`, `.env.toml`, or global `env.toml` files, follow
[legacy configuration recovery](../troubleshooting.md#shine-reports-an-old-configuration-file-after-upgrading-to-040).

## Directory and source precedence

Relative `--config-dir`, `SHINE_CONFIG_DIR`, and `SHINE_PRESETS` paths use the directory where you
invoke Shine.

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

An external preset or overlay root may be a symbolic link; changing its destination requires
review again. Broken root links are errors. Links inside the preset tree are not included;
use ordinary files and directories. Special files such as sockets or devices are rejected.

External Shell presets default to `snapshot` mode: source edits take effect only after
`shine update` and `shine upgrade`. Use `external_shell_mode = "live"` only for development;
source edits then take effect on the next invocation and require development trust. Changes to
`target`, `runtime`, `transforms`, or `env` still need `shine upgrade` to refresh installed commands.

After upgrading Shine, refresh existing Live Bun commands with `shine upgrade`. Commands that use
transforms keep the configuration directory chosen at installation, while project overrides follow
the caller's working directory.

An overlay replaces files at matching relative paths over the chosen base instead of replacing the
whole tree. `presets_overlay_dir` and `presets_overlay_git` are mutually exclusive; use
`shine preset overlay link` to configure them safely. `presets_overlay_git` and
`presets_overlay_git_branch` are global-only keys in `config.toml`; run `overlay link --git`
outside any Shine project. Projects can use `presets_overlay_dir` for a local overlay.
A Git overlay becomes active after the link command clones it, or after a successful
`shine preset pull` when configured manually. Later pulls forcibly mirror its checkout, so never
edit `~/.shine/overlay/` directly.

## Environment value precedence

Later layers override earlier values:

1. built-in defaults (runtime fallback only);
2. active overlay `shine.env.toml` (shared baseline);
3. global `config.toml` `[env]`;
4. global `shine.env.toml`;
5. project `shine.config.toml` `[env]`;
6. project `shine.env.toml`.

`SHINE_CONFIG_DIR` changes the global root from `~/.shine/`. An explicitly saved value stays in
effect even when it equals a default; delete it from its layer to inherit again.
A lone `shine.env.toml` does not define a project. Only the nearest `shine.config.toml` defines the
project root and its env file; ancestor projects are not merged.

Use `shine env explain KEY` for the value source, description source, override chain, and default
write target. It hides all values unless `--reveal` is given and never decrypts. For older
installations, see [upgrade and rollback steps](../guides/upgrading.md#from-23x-or-earlier-environment-layering).

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
# age_recipients = ["age1tag1qexample...", "age1qteammate..."]
```

Sources merge in list order. Current process variables win by default;
`env.override_process_env = true` lets workspace values replace them.

Workspace encryption settings override global defaults. For `shine env secret seal`, explicit
`--backend` and `--recipient` options take priority over workspace settings. Keep shared team
recipient lists in `[env.encryption]`. Recipients are public information; private identity files
and machine-local `age_identity`/`age_identities` paths must stay out of the shared repository.

Create a source with plaintext entries and secrets to enter during sealing:

```toml
version = 1

[plain]
PUBLIC_VALUE = "example"

[secret]
API_TOKEN = false
```

| Source entry | Meaning |
| --- | --- |
| `[plain]` string | A non-secret value |
| `[secret]` value `false` | Prompt for the value during the next `shine env secret seal` |
| `[secret]` value `true` | Retain a value already sealed in this source |
| `[secret]` string | Plaintext waiting to be sealed; do not commit it |
| `[payload].data` | Encrypted content written by Shine; do not fill it in manually |

Sealing encrypts pending values and changes their entries to `true`. `shine env run` combines
plain and decrypted values in source order.

`shine env run --with KEY[=ALIAS]` injects a value from current Shine `[env]`, preferring
`KEY_SECRET` and then `KEY`. Explicit injection overrides workspace and process values.

## Managed directories

These paths are under the global root, normally `~/.shine/`:

| Path | Use |
| --- | --- |
| `config.toml` | Global settings |
| `shine.env.toml` | Global environment overrides |
| `bin/` | Installed command entries; keep this directory on `PATH` |
| `presets/` | Runtime presets, grouped into `app/`, `shell/`, and `sys/` |
| `overlay/` | Shine-managed Git overlay; edit its upstream repository, not this copy |

Shine also keeps installation records and recovery files here. Do not delete them manually and
expect Shine to rediscover old installations. Use the corresponding `uninstall --dry-run` and
`uninstall` commands. For leftover `http/` files from the retired service, follow
[HTTP service migration](../guides/upgrading.md#retire-an-older-local-http-service).

## Hybrid workspace encryption

`env.encryption.backend = "hybrid"` requires both workspace `gpg_recipients` and
`age_recipients`, each with a nonblank entry. GPG entries must be locally available
full 40-hex primary public-key fingerprints. Global recipients are not merged.
Merely configuring both lists does not enable hybrid.

Set `hybrid_decrypt_backend = "gpg"` or `"age"` in global `config.toml`.
Local workspace operations may override this field in `shine.config.local.toml`
next to the workspace; an absent file or field inherits the global preference.
Shared `shine.config.toml` cannot override or save it. Ignore the personal file in
Git; it accepts only this field and rejects malformed or unknown fields. The
preference applies only to hybrid decryption and does not authorize secret access.
SSH broker uses the decrypting machine's choice, not remote personal configuration.
Global `secret_backend` accepts only `gpg` or `age`.

See the [environment guide](../guides/environment.md) for setup and authorization requirements.

## Workspace file groups

Workspace version 3 supports `file_groups.<name>.files`; version 2 does not.

| Setting | Requirement |
| --- | --- |
| Group name | Letters, digits, hyphens, and underscores; each group must contain files |
| `source` | Encrypted file path |
| `target` | Plaintext file path |
| Paths | Relative to the workspace directory; `{mode}` is supported |
| `env.files` | Optional when file groups are present |
| Encryption | Shared `[env.encryption]` settings, without per-group or per-mode overrides |

See [temporary secret files](../guides/environment.md#workspace-secret-files) for sealing,
execution, export, path restrictions, and plaintext cleanup.
