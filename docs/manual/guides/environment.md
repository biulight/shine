---
title: Manage environment variables and secrets
sidebar_position: 5
---

# Manage environment variables and secrets

Shine stores variables used by preset templates and can seal sensitive project-environment values
with GPG or age. Sealed secrets can be injected into local child processes or requested for a remote
command through the SSH Secret Broker while decryption remains local. Never put real secrets in a
public repository or documentation example.

Choose the workflow you need:

- [Set a preset variable](#inspect-and-set-values), such as a proxy address.
- [Provide values to one command](#provide-values-to-one-command) without changing your terminal.
- Encrypt secrets with [GPG](#encrypt-values-with-gpg) or [age](#use-age-identities).
- [Share project environments](#use-layered-project-environments) or [import existing dotenv files](#initialize-a-workspace-from-dotenv).
- [Use encrypted files during a build](#workspace-secret-files) or [provide secrets remotely](#provide-variables-and-secrets-to-remote-commands).

Keep private identities and unsealed secrets out of version control. Only approve decryption for
commands you trust; the receiving command and other processes with sufficient permissions can read
the plaintext.

## Inspect and set values

Choose commands for the operation you need; do not run this as a set/delete sequence:

| Operation | Command |
| --- | --- |
| List variables and their sources | `shine env list` |
| Read the effective value | `shine env get HTTP_PROXY_PORT` |
| Inspect precedence and sources | `shine env explain HTTP_PROXY_PORT` |
| Save a value | `shine env set HTTP_PROXY_PORT 6152` |
| Delete the selected stored value | `shine env delete HTTP_PROXY_PORT` |

`PROXY_NO_PROXY` controls the `NO_PROXY` and `no_proxy` values set by `setproxy` and defaults to
`localhost,127.0.0.1,::1`. After changing it or another proxy variable, `shine update` marks the
installed `proxy` shell preset as updatable; run `shine upgrade` to apply the value.

The built-in image commands use `IMAGE_QUALITY=80`, `IMAGE_MAX_WIDTH=1920`, and
`IMAGE_MAX_HEIGHT=1080` by default. Override them for one run with `--quality`, `--width`, or
`--height`, or keep different machine-local defaults with `shine env set`.

`shine env list` hides sensitive values; `shine env explain KEY` hides all values and shows which
file supplies the effective value. Use `--reveal` only in a trusted terminal; it does not decrypt
secrets. Injection prefers `KEY_SECRET` over `KEY`, so changing a plaintext value may not change
what a command receives.

If Shine reports invalid TOML, correct the indicated file or field before retrying the edit.

Global `~/.shine/config.toml` and project `shine.config.toml` support either a string or a value with
a description:

```toml
[env]
HTTP_PROXY_PORT = "6152"
API_URL = { value = "https://api.example.com", description = "API endpoint" }
```

`value` behaves exactly like the short string form; `description` appears in `shine env list`.
Running `shine env set API_URL https://api.example.org` on a detailed entry updates `value` and preserves the
description.

Write targets depend on options and cwd, independently of the effective source:

| Options | Target |
| --- | --- |
| None | Active project `shine.config.toml` `[env]`, otherwise global config |
| `--global` | Global `config.toml` `[env]` |
| `--project` | Nearest project `[env]`; fails without an existing project |
| `--env-file` | Selected or default scope's `shine.env.toml` |
| `--overlay` | Active manual overlay's `shine.env.toml` |

Scope options are mutually exclusive. `--overlay --env-file` selects the same overlay file. Scripts
should specify both scope and file options to avoid cwd dependence. With no target options, a key
defined in a higher layer is protected, even if values match. Explicit targets allow maintenance of
shadowed entries and report the saved file, current effective source, and whether the value changed.

Choose the operation and storage location you need:

| Operation | Command |
| --- | --- |
| Write global configuration | `shine env set HTTP_PROXY_PORT 7890 --global` |
| Write project configuration | `shine env set HTTP_PROXY_PORT 8080 --project` |
| Write the project env file | `shine env set HTTP_PROXY_PORT 8081 --project --env-file` |
| Write the manual overlay | `shine env set HTTP_PROXY_PORT 6152 --overlay` |
| Delete from the global env file | `shine env delete HTTP_PROXY_PORT --global --env-file` |
| Save an encrypted copy of the effective `MY_TOKEN` as `MY_TOKEN_SECRET` in the project env file | `shine env secret encrypt --from MY_TOKEN --set MY_TOKEN_SECRET --project --env-file` |

The encryption command above preserves the plaintext `MY_TOKEN` in its original layer.

Deletion removes only an existing entry in the selected file; an absent local entry is an error.
It restores inheritance, never disables a variable, removes other layers, or also deletes
`KEY_SECRET`. Empty strings remain valid values. Managed Git overlay mirrors reject writes: edit
upstream, use a manual maintenance directory, or choose local overrides. These three write entries
reject the retired `--force` without writing and suggest the correct replacement options.

Avoid editing configuration while an env write is in progress. If Shine reports a concurrent
change, inspect the file and retry. A message saying the file was saved but its effective state
could not be verified means the write happened; query the value again before making another edit.

Writes preserve comments and unrelated entries, and reject symbolic links and special files.
Unix files use owner-only permissions, but unencrypted values remain plaintext.

Global, overlay, and project `shine.env.toml` files omit the `[env]` header and support both forms:

```toml
HTTP_PROXY_PORT = "7890"
PROXY_HOST = { value = "127.0.0.1", description = "Local proxy host" }
```

For accepted value formats, descriptions, and precedence, see the
[configuration reference](../reference/configuration.md#environment-entry-formats-and-descriptions).

After changing a value used by template rendering, run:

```bash
shine upgrade
```

## Provide values to one command

Use `--with` to provide values without changing the current terminal or creating workspace files:


| Operation | Command |
| --- | --- |
| Inject under the same name | `shine env run --with MY_TOKEN -- bun run build` |
| Rename for the child process | `shine env run --with MY_TOKEN=API_TOKEN -- bun run build` |
| Inject multiple variables | `shine env run --with TOKEN_A --with TOKEN_B=OTHER_TOKEN -- bun run build` |
| Skip workspace loading | `shine env run --no-workspace --with MY_TOKEN -- bun run build` |

Each key prefers `<KEY>_SECRET` and falls back to plaintext `<KEY>`. The name after `=` is the child
process variable. Explicit `--with` values override the current process and workspace.

`--no-workspace` skips `shine.workspace.toml` discovery completely and merges only the current
process and explicit `--with` values. It cannot be combined with `--workspace` or `--mode`.

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

The wrapper keeps using the Shine configuration directory chosen at installation, even if you
later change `SHINE_CONFIG_DIR`. Re-run `env proxy install` to use another configuration directory.
Project rules still follow the directory where you run the command.

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

Proxy only a command you trust, using a bare name containing ASCII letters, numbers, `-`, `_`, or
`.`. Put `~/.shine/bin/` early in `PATH`. The target must be an executable outside Shine's command
directory and cannot be another Shine wrapper or Shine itself.

On Windows, use `gh`, not `gh.exe`: `.exe`, `.com`, `.bat`, `.cmd`, and `.ps1` suffixes are rejected.
Previously installed suffixed names can still be removed with `shine env proxy uninstall <COMMAND>`.

Shine preserves conflicting files during installation or removal. Inspect reported conflicts before
retrying. If an operation fails or is interrupted, check the wrapper and configuration before using
it again; do not assume recovery completed.

Rules default to global `~/.shine/config.toml`. Inside a project with `shine.config.toml`, add
`--project` to scope the rule; a project rule for the same command overrides the global one.
Outside that project, the shim uses the global rule if present; otherwise it forwards to the real
command without injecting values:

```bash
shine env proxy install gh --with GH_TOKEN --project
shine env proxy list
```

Disable injection temporarily while retaining the wrapper; the command will run without injected values:

| Operation | Command |
| --- | --- |
| Disable the proxy | `shine env proxy disable gh` |
| Enable the proxy | `shine env proxy enable gh` |
| Disable the project proxy | `shine env proxy disable gh --project` |

Without `--project`, enable/disable changes the global rule even inside a project. A project rule
can still override it. If Shine reports that the rule was saved but its effective state could not
be verified, inspect the configuration and run `shine env proxy list` again.

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

Direct secret forwarding suits temporary work on a trusted host, but the remote login shell and
same-account processes may read the plaintext. Use the SSH Secret Broker when the decryption key
must stay local and only a selected remote child should receive the value. The remote administrator
or same-account processes can still inspect that child.

For setup commands, local policies, enrollment, and the complete security boundary, see
[SSH sessions, secret brokering, and file transfer](./ssh-transfer.md#provide-secrets-to-remote-commands-on-demand).

## Encrypt values with GPG

Install the selected encryption backend (`gpg` or `age`) and any identity plugins it requires.

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

Encryption needs only recipient public keys. Decryption needs the corresponding private key.
If that key is on a YubiKey, connect it and follow any PIN or touch prompts. `decrypt` prints
plaintext, so run it only in a trusted terminal. Encryption leaves the original plaintext variable
in place; remove it explicitly when you no longer need it.

Legacy `gpg_key_id` is deprecated. Preview and apply migration with
`shine state migrate --dry-run` and `shine state migrate`; a workspace using the old
`[env.encryption].recipient` is prompted to migrate when `env run` or `env secret seal` needs it.

Export a value into the current shell:

Choose whether to keep or rename the variable:

```bash
export_code="$(shine env secret export MY_TOKEN)" && eval "$export_code"
export_code="$(shine env secret export MY_TOKEN --as API_TOKEN)" && eval "$export_code"
```

In PowerShell, evaluate the generated code only after a successful export:

```powershell
$exportCode = shine env secret export MY_TOKEN --as API_TOKEN
if ($LASTEXITCODE -eq 0) { Invoke-Expression $exportCode }
```

After installing the `utils` shell preset, you can also use
`shine-env-export MY_TOKEN --as API_TOKEN`. Exported values remain available in the current shell;
use [one-command injection](#provide-values-to-one-command) when only one command needs them.
Values containing NUL cannot be exported as environment variables.

## Use age identities

Use age to encrypt project secrets for one or more recipients. Existing GPG ciphertext remains
readable without conversion.

First ensure age 1.3 or newer is installed and available on `PATH`. Create a normal software
identity and record its recipient:

```bash
shine env secret identity init
shine env secret identity list
```

A normal identity defaults to `~/.shine/age/identity.txt`. Use `--output` to choose another path,
then add it to your `age_identity` or `age_identities` configuration as shown by the command.
Paths used in TOML must be valid UTF-8. `identity list` shows configured identity paths and recipients.

Replacing an existing identity requires `--force`. Keep a secure backup if you still need it to
decrypt older data. Unix identity files are accessible only to their owner.

### Use Touch ID on macOS

On macOS, choose a Secure Enclave identity instead when decryption should require local user
authorization. It also requires `age-plugin-se`; Homebrew can install both dependencies:

```bash
brew install age age-plugin-se
```

Run the Touch ID form instead of the normal identity initialization above, then record its
recipient:

```bash
shine env secret identity init --touch-id
shine env secret identity list
```

`--touch-id` is macOS-only. Decryption requires the original Mac's Secure Enclave and authorization
under the selected access policy, using Touch ID by default. Copying the identity file to another
machine is normally insufficient to decrypt. Cancel unexpected authorization prompts and inspect the command that caused them. For the
security boundary when AI agents can run local commands, see
[Protect environment secrets when using AI agents](./agent-secret-safety.md#what-touch-id-improves).

New Touch ID identities use an `age1tag...` recipient. Age 1.3 can encrypt to that public recipient
on macOS, Linux, or Windows without installing `age-plugin-se`; only decryption needs the plugin and
the original Mac. Tagged recipients trade that portability for discoverability: someone who already
knows the recipient can test whether a ciphertext targets it.

Configure machine-wide defaults in `~/.shine/config.toml`:

```toml
secret_backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
age_identities = ["C:/Users/<user>/AppData/Local/age-plugin-phone/identity-....txt"]
```

Use `age_identity` for one identity and `age_identities` for additional identities, including
hardware-backed ones. Keep these files on the machines that decrypt the secrets.

A project-team recipient list belongs in `[env.encryption]` in the project's
`shine.workspace.toml`. It can be committed and overrides global defaults without affecting other
projects. Never commit the private `age_identity`.

Select a backend and recipients for one command:

```bash
shine env secret encrypt --backend age -r age1tag1qexample... -r age1qteammate... --from MY_TOKEN
shine env secret seal --backend age -r age1tag1qexample... -r age1qteammate...
```

`-r/--recipient` is repeatable for both GPG and age.

Older Secure Enclave identities may have an `age1se...` recipient, which requires
`age-plugin-se` even on a machine that only encrypts. From the project containing the workspace,
preview and apply the public-recipient conversion:

| Operation | Command |
| --- | --- |
| Preview legacy-state migration | `shine state migrate --dry-run` |
| Apply after reviewing the preview | `shine state migrate` |

Migration converts configured `age1se...` recipients to equivalent `age1tag...` recipients without
decrypting secrets. Reseal afterward to use the new recipients. Existing ciphertext remains
unchanged and can still be decrypted.

### Add recipients to existing workspace secrets

Changing `age_recipients` alone does not update existing ciphertext. Keep the existing recipients
and append the new members' public recipients in `shine.workspace.toml`:

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1existing...", "age1newmember...", "age1backup..."]
```

Replace every placeholder with an actual recipient, then run this on a machine whose configured
identity can still decrypt the old payloads:

```bash
shine env secret seal
```

`seal` decrypts the old payloads and encrypts them again for the complete current recipient list.
Even when every `[secret]` entry is `true`, its value is retained and re-encrypted; leave these
entries as `true`, without exporting plaintext or entering the values again. Only the new members'
public recipients are needed, not their private identities. Decrypting old payloads may still
require Touch ID, phone authorization, or another identity-specific prompt.

Without a file argument, `seal` processes existing sources referenced by the workspace across its
configured modes, including `default_mode`. Use `shine env secret seal <FILE>` to process only one
source. If sealing stops with an error, some files may already be updated; resolve the reported
error and rerun it before sharing the result.

Commit the updated workspace configuration together with the resealed shared source files. Each
listed recipient can independently decrypt the new ciphertext; have new members verify access on
their own machines. New recipients cannot decrypt old ciphertext in Git history. Likewise, removing
a recipient and resealing does not revoke its access to historical ciphertext or rotate the real
upstream credentials; rotate exposed credentials at their source when necessary.

### Experiment with phone authorization on Windows and macOS {#experiment-with-phone-authorization-on-windows}

The [`age-plugin-phone`](https://github.com/biulight/age-plugin-phone) integration is experimental.
Use synthetic or disposable data, not real or production secrets. Keep an independently tested
recovery recipient that does not depend on the same phone, computer, or plugin state.

Install age 1.3+ and matching desktop-plugin and phone-app versions that support tagged recipients.
Follow the plugin's [Windows quick start](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-beta-quickstart.md)
or [macOS quick start](https://github.com/biulight/age-plugin-phone/blob/main/docs/macos-quickstart.md)
for hardware requirements, supported transports, installation, and recovery. Do not import a
private test-signing root into your system trust store.

For Wi-Fi pairing, first open **Pair · Wi-Fi** in the phone app, then run:

```powershell
shine env secret identity init --phone --label "Work laptop" --transport auto
```

`auto` tries Wi-Fi discovery first. Without a responding phone it selects Developer USB/ADB on
Windows or QR on macOS. For this experimental workflow, keep the Wi-Fi listener ready or select
Android ADB explicitly; QR with tagged recipients is outside the documented Beta validation scope.
For USB, run the desktop command first, wait for it to request the phone connection, then choose
**Pair · USB** on the phone:

```sh
shine env secret identity init --phone --label "Work Mac" --transport adb --adb-serial SERIAL
```

Use `--adb-serial` when more than one ADB device is connected. If pairing reports ambiguous devices
or a discovery error, resolve it before retrying. Do not start another pairing to recover an
interrupted setup; follow the plugin's `setup --resume` or `setup --cleanup` instructions.

After fingerprint confirmation, Shine adds the identity path to the user's global `age_identities`.
If the current project overrides `age_identity` or `age_identities`, resolve that override before
pairing. If pairing succeeds but saving configuration fails, use the configuration printed by
Shine; the pairing remains active.

The command does not change `secret_backend` or add recipients. Put the printed recipient and your
verified recovery recipient in the project's `shine.workspace.toml`:

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1tag...", "age1..."]
```

The default tagged recipient allows encryption with age 1.3+ without a phone plugin or prompt on
the encrypting machine. Decryption still requires the plugin, matching phone app, and fresh biometric
authorization on the phone. Someone who knows the tagged recipient can test whether a ciphertext targets it.
If you need the plugin's `age1phone...` format, use
`shine env secret identity init --phone --recipient-type phone`; that format requires the plugin
on every encrypting machine too.

For an existing pairing, verify tag support in the desktop plugin and phone app, then export its
public tagged recipient without pairing again:

```powershell
age-plugin-phone recipients -i <IDENTITY_STUB> --recipient-type tag
```

Replace the corresponding recipient in your configuration, retain the recovery recipient, and
[reseal the workspace secrets](#add-recipients-to-existing-workspace-secrets). Exporting the
recipient alone does not update existing ciphertext; `shine state migrate` does not convert phone
recipients. Resealing may require phone authorization and does not revoke access to old ciphertext.

For later Wi-Fi decryption, enable **Wi-Fi auto-listen** and keep the phone app in the foreground.
Set `AGE_PLUGIN_PHONE_MESSAGES=1` when you need plugin guidance. If multiple configured phone
identities cause decryption to fail, check for a plugin update or temporarily select only the
matching identity; keep pairing and recovery state intact.

If AI agents can run commands on your machine, read
[Protect environment secrets when using AI agents](./agent-secret-safety.md) before use.

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
# age_recipients = ["age1tag1qexample...", "age1qteammate..."]
```

Later files override earlier ones. `{mode}` expands from `--mode`, or from `default_mode` when the
flag is omitted. A source can contain plaintext and secrets pending sealing:

```toml
version = 1

[plain]
VITE_APP_NAME = "Example App"

[secret]
DATABASE_URL = false
API_TOKEN = false
```

`false` prompts for a value when you seal the file. Shine encrypts it, changes the entry to `true`,
and saves the ciphertext; do not create `[payload]` yourself. Existing `true` entries retain their
values when resealed. You can also supply a string to seal, but it remains plaintext until sealing
and must not be committed.

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

Ignore personal overrides:

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

Never commit a source containing unsealed strings. Before committing, inspect `[secret]` and confirm
every sealed entry is `true`. See the [configuration reference](../reference/configuration.md) for
file formats and precedence.

## Initialize a workspace from dotenv

At a project root with Vite-style `.env` files, generate a Shine workspace and TOML sources:

| Operation | Command |
| --- | --- |
| Preview importing dotenv files | `shine env workspace init --from-dotenv --dry-run` |
| Apply the reviewed import | `shine env workspace init --from-dotenv` |

The command reads `.env`, `.env.local`, `.env.<mode>`, and `.env.<mode>.local`, discovers modes, and
preserves that precedence. Source dotenv files are unchanged. Existing targets are preserved unless you add `--force`. Import selected modes with repeatable `--mode`:

```bash
shine env workspace init --from-dotenv --mode development --mode production
```

`--dry-run` previews the import without creating or replacing files.

Mark known sensitive keys for `[secret]`, then configure recipients and seal. Unmarked values are
imported as plaintext; never accidentally commit credentials as ordinary configuration.
Imported secrets also remain plaintext until sealing. On Unix, generated workspace and source files are accessible only to their owner.

```bash
shine env workspace init --from-dotenv --secret DATABASE_URL
shine env secret seal
```

To preserve dotenv semantics, values containing `$` (including `$BASE_URL` and `${BASE_URL}`) or
escaped double-quoted content are rejected, even inside quotes. Resolve interpolation before
importing; keep literal dollar signs in a manually authored Shine source instead. Single-line
backtick-quoted values preserve literal hashes, quotes, backslashes, and whitespace. Multiline
quoted values are rejected. An unquoted `#`
starts a comment; quote values that contain a literal `#`.

## Export a workspace to dotenv

Export one fully resolved mode when another tool needs a conventional dotenv file, or when you want
to stop using Shine env:

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local
```

`--format` is required. Export includes the selected workspace mode, without current process
variables or `--with` values. By default it exports only winning `[plain]` entries and does not decrypt payloads. If a
later secret declaration shadows an earlier plain value, that old plain value is omitted.

Include sealed secrets only when the destination really needs a complete runnable environment:

```bash
shine env workspace export \
  --format dotenv \
  --mode production \
  --output .env.production.local \
  --include-secrets
```

This writes plaintext secrets. Keep the output out of version control; on Unix, it is readable
only by its owner. Existing destinations are preserved unless you add `--force` to replace them.
`--dry-run` previews the destination and non-secret variables without printing values, decrypting
secrets, or writing files, even when `--include-secrets` is present.

The exported file has no Shine metadata or runtime dependency. To leave Shine env, export and test
each required mode, add secret-bearing outputs to `.gitignore`, remove `shine env run` wrappers, and
only then archive or delete `shine.workspace.toml` and its `*.shine.toml` sources yourself. Export
never deletes those source files.

Export preserves literal backslashes, quotes, and newlines using dotenv quoting. Values containing
`$`, carriage returns, NUL bytes, or no safe quote delimiter are rejected before writing or replacing
the output. For those values, continue using `shine env run`, or prepare an output specifically for
your target tool. Always test the exported file with that tool before switching over.

## Share secrets with GPG and age users {#share-a-payload-across-gpg-and-age-members}

Before converting a workspace from Shine 2.0.x, upgrade all readers, including the local SSH
broker machine, to a release supporting hybrid encryption. Older readers cannot read `hybrid:`
ciphertext. Sealing requires GnuPG 2.2–2.5 and age 1.3+; readers need only the backend they select.
Existing untagged GPG and `age:` ciphertext stay readable; ordinary reads never migrate
files. Explicitly configure the workspace (replace the public placeholders):

```toml
[env.encryption]
backend = "hybrid"
gpg_recipients = ["<full 40-hex primary fingerprint>"]
age_recipients = ["age1..."]
```

Run `shine env secret seal --workspace shine.workspace.toml` on a machine that can decrypt the
existing secrets and has both encryption tools. Either recipient group can then decrypt the shared
secrets independently. Use full primary-key fingerprints for GPG recipients; groups and partial
names are not accepted, and implicit recipients from local GPG settings are not included.

Each use still follows the selected backend's authorization requirements. If decryption fails or
you cancel, the command stops instead of trying another backend. After upgrading or changing
workspace inputs, the first run may ask you to authorize more source files; you do not need to
reseal existing sources just for that reason.

For a personal project preference, create `shine.config.local.toml` alongside the
selected `shine.workspace.toml` (also when using `--workspace`):

```toml
hybrid_decrypt_backend = "age"
```

The value can be `gpg` or `age`. Add `/shine.config.local.toml` to the project's `.gitignore`.
Local run, seal, and export commands use it; otherwise they use the global preference or ask you
to choose an available backend. A program that can edit this file can change which authorization
method is used. SSH broker requests use the decrypting machine's own choice.

Seal regular source files, not symbolic links or, on Unix, files with multiple hard links.
Do not edit workspace or source files while sealing, including while approving hardware prompts.
If sealing fails, earlier files may already be updated and pending plaintext may remain. Inspect
the reported files before retrying or committing. Do not delete `.shine-seal.lock` while sealing
is active.

## Temporary secret files {#workspace-secret-files}

Before enabling temporary secret files, upgrade all clients reading the same workspace to
Shine 2.5 or later.

Use workspace version 3 to keep signing keystores, password files, and service credentials encrypted
in Git. Organize the files into named groups: `source` is the encrypted file and `target` is where
ordinary tools will read its decrypted contents. Select one or more groups with `--file-group`.

Replace the recipient placeholder with a real public recipient. Identities stay outside the repository:

```toml
version = 3

[env]
default_mode = "staging"
modes = ["staging", "production"]

[env.encryption]
backend = "age"
age_recipients = ["<actual age recipient>"]

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

Paths are relative to the workspace directory. `{mode}` uses `--mode`, or `env.default_mode` when
omitted; the mode must be declared in the workspace. Missing files stop the operation rather than
falling back to another environment. If a workspace has only file groups and no `env.files`,
`env run` requires an explicit `--file-group`.

### Prepare and seal the files

In a Git repository, add every plaintext target to `.gitignore` before use, for example
`.android-signing/` and `app/.credentials/`. Targets must be untracked and ignored in their own
repository, including nested repositories. Shine does not edit `.gitignore`; resolve Git inspection
errors before continuing. File groups also work outside Git repositories.

After preparing the plaintext target files, preview and seal them:

```bash
shine env secret seal --mode production --file-group android-signing --file-group service-account --dry-run
shine env secret seal --mode production --file-group android-signing --file-group service-account --remove-plaintext
```

Sealing reads each `target` and writes its encrypted `source` using the workspace's encryption
settings. It keeps plaintext unless you add `--remove-plaintext`. That option needs a usable
decryption identity and verifies all written ciphertext before deletion; verification failure or
cancellation before deletion preserves all original plaintext. A later write or deletion failure
can leave partial results, so inspect the reported files before retrying. Sealing file groups
requires plaintext targets, including when changing recipients.

### Run a command with temporary files

```bash
shine env run --mode production --file-group android-signing --file-group service-account -- pnpm android:build
```

Shine decrypts all selected files before starting the command and refuses to overwrite existing
targets. When the command ends or is interrupted normally, Shine attempts to remove the files it
created. Files replaced during the run are preserved and reported; inspect cleanup warnings for
plaintext left behind.

Do not start detached tasks that need the files after the command ends; remaining managed child
processes are terminated. Changes to decrypted files are not automatically encrypted back into
the repository.

### Keep decrypted files for other tools

To use ordinary tools without wrapping each command, export the selected groups:

```bash
shine env workspace export --format files --mode production --file-group android-signing --file-group service-account --dry-run
shine env workspace export --format files --mode production --file-group android-signing --file-group service-account
```

`--mode` is required. This exports files to their configured targets, not environment variables.
All selected files must decrypt before writing starts. Existing targets are preserved: inspect
and move them before retrying. The `files` format has no `--force`, `--output`, or
`--include-secrets` option. Workspace configuration and encrypted sources stay unchanged.

Successful exports remain on disk until you remove them. `env run --file-group` refuses those
existing targets and will not clean them up. If export fails or is interrupted, Shine attempts to
remove files and empty directories created by that export, preserves replacements, and reports
possible leftovers. Check these warnings before retrying. Never commit the plaintext.

### Limits and precautions

- Plaintext files are accessible only to the current user; Windows requires a filesystem that
  supports these private permissions. The command and other processes with sufficient local
  permissions can still read them.
- Paths must remain inside the workspace. Links, overlapping mappings, and mappings that conflict
  with the workspace definition are rejected. Use distinct paths for every source and target.
- A plaintext file can be at most 16 MiB, with 64 MiB total per invocation. Encrypted files are
  limited to 32 MiB each and 128 MiB total. Use Shine to read these encrypted files; they are not
  native `.age` or `.gpg` files.
- `--dry-run` checks configuration and metadata without reading plaintext, decrypting, writing,
  or starting commands. It cannot verify ciphertext integrity or private-key availability.
- Select groups explicitly. `--file-group` cannot accompany `seal <FILE>`, or `env run` with
  `--no-workspace` or `--secret-broker`. Persistent file export supports `--secret-broker` below.
- Force-killing Shine or losing power can leave plaintext behind. Inspect targets before retrying.
  Deleting files is not secure erasure, and removing a recipient does not revoke access to old
  ciphertext in Git history.

### Export through the SSH broker

Add `--secret-broker` to `env workspace export --format files` to request local authorization
and decryption from an enabled `shine ssh --secret-broker` session. This option requires explicit
file groups and a matching file policy; it is unavailable for dotenv export. Without the flag,
manual export continues to use the executing machine's backend. See
[SSH file authorization](ssh-transfer.md#file-secret-broker) and
[automatic App secret installation](app-presets.md#managed-secret-files).

## Manual upgrade and rollback for env layering

When upgrading from 2.3.x or earlier, follow [Upgrades and migration](./upgrading.md#from-23x-or-earlier-environment-layering)
to check value precedence, update scripts, and prepare for rollback.
