---
title: Manage environment variables and secrets
sidebar_position: 5
---

# Manage environment variables and secrets

Shine stores variables used by preset templates and can seal sensitive project-environment values
with GPG or age. Sealed secrets can be injected into local child processes or requested for a remote
command through the SSH Secret Broker while decryption remains local. Never put real secrets in a
public repository or documentation example.

Secret operations live under `shine env secret`. Workspace-based `shine env run` and
`env run --with` inject values on demand. For remote commands, first choose between direct forwarding
and brokered on-demand decryption as described below.

## Inspect and set values

```bash
shine env list
shine env get HTTP_PROXY_PORT
shine env set HTTP_PROXY_PORT 6152
shine env delete HTTP_PROXY_PORT
```

`PROXY_NO_PROXY` controls the `NO_PROXY` and `no_proxy` values set by `setproxy` and defaults to
`localhost,127.0.0.1,::1`. After changing it or another proxy variable, `shine update` marks the
installed `proxy` shell preset as updatable; run `shine upgrade` to apply the value.

The built-in image commands use `IMAGE_QUALITY=80`, `IMAGE_MAX_WIDTH=1920`, and
`IMAGE_MAX_HEIGHT=1080` by default. Override them for one run with `--quality`, `--width`, or
`--height`, or keep different machine-local defaults with `shine env set`.

`shine env list` hides sensitive values by default. Use `--reveal` only in a trusted terminal. Output
is grouped by the effective source—`config.toml`, global override, overlay, or project override—so
you can identify which value wins. Values normally live in the active configuration's `[env]` table.

Global `~/.shine/config.toml` and project `shine.config.toml` support either a string or a value with
a description:

```toml
[env]
HTTP_PROXY_PORT = "6152"
MY_API_TOKEN = { value = "<token>", description = "Token for the internal API" }
```

`value` behaves exactly like the short string form; `description` appears in `shine env list`.
Running `shine env set MY_API_TOKEN <new-value>` on a detailed entry updates `value` and preserves the
description.

When a global, overlay, or project `shine.env.toml` already overrides a key, `set`, `delete`, and
`env secret encrypt --set` refuse to write a lower-priority value that would have no effect. Add
`--force` only when you intend to modify that override file:

```bash
shine env set HTTP_PROXY_PORT 7890 --force
shine env delete HTTP_PROXY_PORT --force
shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET --force
```

For a mirror managed by `shine preset overlay link --git`, the next `shine preset pull` discards a
forced local write. Maintain that value in the upstream overlay repository instead.

Global, overlay, and project `shine.env.toml` files omit the `[env]` header and support both forms:

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "Local proxy host" }
```

A detailed entry overrides both value and description. A string overrides only the value and keeps a
description from lower-priority configuration or the preset catalog. Invalid entries—numbers,
arrays, or a table without `value`—fail explicitly.

After changing a value used by template rendering, run:

```bash
shine upgrade
```

## Encrypt values with GPG

First make sure local `gpg` can use the recipient public key. For a private key on YubiKey, see the
Chinese guide
[在 macOS 和 Windows 使用 YubiKey OpenPGP](https://blog.biulight.top/timeline/knowledge/yubikey-openpgp).
Set one or more default recipients in `~/.shine/config.toml`:

```toml
gpg_recipients = ["user@example.com", "team-backup@example.com"]
```

Encrypt an existing plaintext value into another key:

```bash
shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET
shine env secret decrypt MY_TOKEN_SECRET
```

Encryption needs only recipient public keys. Decryption requires the YubiKey containing the private
key and may prompt for a PIN or touch.

Legacy `gpg_key_id` is deprecated. Preview and apply migration with
`shine state migrate --dry-run` and `shine state migrate`; a workspace using the old
`[env.encryption].recipient` is prompted to migrate when `env run` or `env secret seal` needs it.

Export a value into the current shell:

```bash
eval "$(shine env secret export MY_TOKEN)"
eval "$(shine env secret export MY_TOKEN --as API_TOKEN)"
```

After installing the `utils` shell preset, `shine-env-export MY_TOKEN --as API_TOKEN` is equivalent.

## Use age and Touch ID

The `age` backend is suitable for committing team ciphertext encrypted to multiple member
recipients. Existing GPG ciphertext needs no migration: legacy untagged ciphertext continues to use
GPG, while new age ciphertext has an `age:` tag.

Install age. On macOS, Touch ID and Secure Enclave identities also require `age-plugin-se`:

```bash
brew install age age-plugin-se
```

Create identities and record their recipients:

```bash
shine env secret identity init
shine env secret identity init --touch-id
shine env secret identity list
```

`--touch-id` is macOS-only and prompts for Touch ID during decryption. A normal identity uses
`age-keygen` and defaults to `~/.shine/age/identity.txt`.

Configure machine-wide defaults in `~/.shine/config.toml`:

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
```

A project-team recipient list belongs in `[env.encryption]` in the project's
`shine.workspace.toml`. It can be committed and overrides global defaults without affecting other
projects. Never commit the private `age_identity`.

Select a backend and recipients for one command:

```bash
shine env secret encrypt --backend age -r age1se1qexample... -r age1qteammate... --from MY_TOKEN
shine env secret seal --backend age -r age1se1qexample... -r age1qteammate...
```

`-r/--recipient` is repeatable for both GPG and age. Removing a recipient does not revoke historical
ciphertext; re-encrypt or reseal it to rotate access. If AI agents participate in development, read
[Protect environment secrets when using AI agents](./agent-secret-safety.md) first.

## Provide values to one command

Use repeatable `--with` without changing the current terminal or creating workspace files:

```bash
shine env run --with MY_TOKEN -- bun run build
shine env run --with MY_TOKEN=API_TOKEN -- bun run build
shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build
shine env run --no-workspace --with MY_TOKEN -- bun run build
```

Each key prefers `<KEY>_SECRET` and falls back to plaintext `<KEY>`. The name after `=` is the child
process variable. Explicit `--with` values override the current process and workspace.

`--no-workspace` skips `shine.workspace.toml` discovery completely and merges only the current
process and explicit `--with` values. It cannot be combined with `--workspace` or `--mode`. Managed
Bun entries use this mode when they need fixed Shine configuration independent of the working
directory.

## Choose one-time injection or a transparent wrapper

Use one-time injection for an occasional sensitive operation. For example, Cargo accepts
`CARGO_REGISTRY_TOKEN` for crates.io when its `cargo:token` credential provider is active, so a yank
can receive the token without leaving it in the shell or keeping a command wrapper enabled:

```bash
shine env run --no-workspace \
  --with CARGO_REGISTRY_TOKEN \
  -- cargo yank my-crate@1.2.3
```

`--with CARGO_REGISTRY_TOKEN` prefers encrypted `CARGO_REGISTRY_TOKEN_SECRET` and injects the
plaintext only into Cargo for this run. Cargo and any descendants it launches can still read the
value. For ordinary persistent Cargo authentication, Cargo recommends an operating-system
credential provider; use Shine injection when you intentionally keep the token encrypted in Shine.
See [Cargo registry authentication](https://doc.rust-lang.org/stable/cargo/reference/registry-authentication.html)
and [`cargo yank`](https://doc.rust-lang.org/stable/cargo/commands/cargo-yank.html).

### Install a transparent wrapper for fixed credential variables

Use a transparent wrapper when a CLI repeatedly needs the same fixed credential variable. Some
CLIs, such as GitHub CLI, read a variable like `GH_TOKEN` instead of accepting it as an argument:

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

Shine creates a same-name shim in `~/.shine/bin/` and records the real command found in `PATH`. The
shim resolves `GH_TOKEN_SECRET` only for its child and falls back to plaintext `GH_TOKEN`. It never
exports the value back to the parent. `--with` is repeatable and accepts `KEY=ALIAS`.

An installed proxy is command-wide, not subcommand-specific. If you deliberately proxy Cargo,
disable injection until it is needed:

```bash
shine env proxy install cargo --with CARGO_REGISTRY_TOKEN
shine env proxy disable cargo

# Later, for the credentialed operation:
shine env proxy enable cargo
cargo yank my-crate@1.2.3
shine env proxy disable cargo
```

While enabled, every Cargo subcommand and any descendant process may inherit the token. Disabling
the rule retains the shim and forwards directly to the real Cargo without decrypting or injecting
values. For an occasional yank, prefer the one-time `env run` form above.

Proxy only an explicitly approved bare command name containing ASCII letters, numbers, `-`, `_`, or
`.`. Make sure `~/.shine/bin/` is early in `PATH` and the target is not another Shine wrapper. Shine
refuses to overwrite a same-name entry it does not own.

Rules default to global `~/.shine/config.toml`. Inside a project with `shine.config.toml`, add
`--project` to scope the rule; a project rule for the same command overrides the global one:

```bash
shine env proxy install gh --with GH_TOKEN --project
shine env proxy list
```

Disable injection temporarily while retaining the shim; the disabled wrapper directly forwards to
the real program:

```bash
shine env proxy disable gh
shine env proxy enable gh
shine env proxy disable gh --project
```

Remove the managed shim and user-level rule when no longer needed:

```bash
shine env proxy uninstall gh
```

If the real executable moves or is replaced, rerun the install command to record its new path.

## Provide variables and secrets to remote commands

Choose according to how widely plaintext may be visible remotely:

| Goal | Local command | Plaintext visibility |
| --- | --- | --- |
| Forward a normal value | `shine ssh --with API_URL dev` | Remote login shell or specified command |
| Decrypt and forward a secret directly | `shine ssh --with-secret API_TOKEN dev` | Remote login shell or specified command |
| Let an authorized remote child request local decryption | `shine ssh --secret-broker ... dev` | Only the approved remote child process |

`--with-secret KEY[=ALIAS]` decrypts local `KEY_SECRET` when establishing the session. It suits
temporary work on a trusted host. The remote login shell and same-account processes may read the
plaintext; this is not an isolated secret channel.

Use the SSH Secret Broker when the private key, age identity, or YubiKey stays local while the remote
project contains sealed workspace ciphertext. The remote side submits a command and secret request;
the local agent checks an allow-list or exact policy, confirms locally, decrypts locally, and injects
plaintext briefly into the approved remote child:

```bash
# Local: permit API_TOKEN requests, with local confirmation for every direct request.
shine ssh --secret-broker --allow-secret API_TOKEN dev

# Remote: inject API_TOKEN only into this child process.
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

The broker never transfers the decryption key or puts plaintext in the remote login shell. The target
child, remote administrator, or malicious same-account process can still read plaintext. Fixed
projects should use a local policy bound to the workspace digest, mode, full command, and releasable
keys. See [SSH sessions, secret brokering, and file transfer](./ssh-transfer.md#provide-secrets-to-remote-commands-on-demand).

## Initialize a workspace from dotenv

At a project root with Vite-style `.env` files, generate a Shine workspace and TOML sources:

```bash
shine env workspace init --from-dotenv --dry-run
shine env workspace init --from-dotenv
```

The command reads `.env`, `.env.local`, `.env.<mode>`, and `.env.<mode>.local`, discovers modes, and
preserves that precedence. Source dotenv files are unchanged. Existing targets are not overwritten
unless you add `--force`. Import selected modes with repeatable `--mode`:

```bash
shine env workspace init --from-dotenv --mode development --mode production
```

Mark known sensitive keys for `[secret]`, then configure recipients and seal. Unmarked values are
imported as plaintext; never accidentally commit credentials as ordinary configuration.

```bash
shine env workspace init --from-dotenv --secret DATABASE_URL
shine env secret seal
```

To preserve dotenv semantics, files containing interpolation such as `${BASE_URL}` or escaped
double-quoted values are rejected. Resolve them to final values before importing. The generated file
includes a documented empty `[secret]` table even when no `--secret` is selected.

## Export a workspace to dotenv

Export one fully resolved mode when another tool needs a conventional dotenv file, or when you want
to stop using Shine env:

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local
```

`--format` is required so the export contract remains explicit. The command merges the workspace
sources in their declared order, but does not include inherited process variables or `--with`
values. By default it exports only winning `[plain]` entries and does not decrypt payloads. If a
later secret declaration shadows an earlier plain value, that old plain value is omitted.

Include sealed secrets only when the destination really needs a complete runnable environment:

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local \
  --include-secrets
```

This writes plaintext secrets. On Unix, a secret-bearing output is created with owner-only `0600`
permissions; keep it out of version control on every platform. Existing outputs are rejected unless
you add `--force`, and `--dry-run` reports the mode, destination, and variable count without printing
values or writing the file.

The exported file has no Shine metadata or runtime dependency. To leave Shine env, export and test
each required mode, add secret-bearing outputs to `.gitignore`, remove `shine env run` wrappers, and
only then archive or delete `shine.workspace.toml` and its `*.shine.toml` sources yourself. Export
never deletes those source files.

## Use layered project environments

At the project root, declare modes, ordered sources, and shared recipients in
`shine.workspace.toml`:

```toml
version = 2

[env]
modes = ["development", "production"]
default_mode = "development"
files = [
  ".env.shine.toml",
  ".env.local.shine.toml",
  ".env.{mode}.shine.toml",
  ".env.{mode}.local.shine.toml",
]

[env.encryption]
gpg_recipients = ["user@example.com", "team-backup@example.com"]
# For age, uncomment and add every member recipient:
# backend = "age"
# age_recipients = ["age1se1qexample...", "age1qteammate..."]
```

Later files override earlier ones. `{mode}` expands from `--mode`, or from `default_mode` when the
flag is omitted. A source can contain plaintext and secrets pending sealing:

```toml
version = 1

[plain]
VITE_APP_NAME = "Example App"

[secret]
DATABASE_URL = true
API_TOKEN = false
SENTRY_TOKEN = "<value to seal>"

[payload]
data = "<GPG ciphertext managed by Shine>"
```

- `true` keeps an existing value in the encrypted payload.
- `false` prompts safely during the next `seal`.
- A string is replaced by `true` after sealing so plaintext does not remain in the file.

Seal pending values and start a command with the merged environment:

```bash
shine env secret seal
shine env run --mode production -- bun run build
```

By default, `seal` processes workspace sources. Pass a file to seal only that source, or use
`--workspace <FILE>` for another workspace. `-r/--recipient` temporarily overrides recipients.

Existing process variables override the workspace by default. With
`env.override_process_env = true`, workspace values win. Explicit `--with` always has highest
precedence.

When usable GPG recipients are configured, `env run` stores an encrypted, mode-specific GPG cache in
the system cache directory. It rebuilds automatically when workspace content, source content, or file
order changes.

Ignore personal overrides:

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

Never commit a source containing unsealed strings. Before committing, inspect `[secret]` and confirm
every sealed entry is `true`. See the [configuration reference](../reference/configuration.md) for
file formats and precedence.
