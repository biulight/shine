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

Shine handles Base64 encoding and decoding internally for both GPG and age secrets; no external
`base64` command is required for these operations. Install the selected encryption backend (`gpg`
or `age`) and any identity plugins it needs. Existing Shine ciphertext needs no migration.
Wrapped Base64 and ASCII whitespace are accepted; malformed encoding, including missing or invalid
padding, is rejected.

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

## Use age identities

The `age` backend is suitable for committing team ciphertext encrypted to multiple member
recipients. Existing GPG ciphertext needs no migration: legacy untagged ciphertext continues to use
GPG, while new age ciphertext has an `age:` tag.

First ensure age 1.3 or newer is installed and available on `PATH`. Create a normal software
identity and record its recipient:

```bash
shine env secret identity init
shine env secret identity list
```

A normal identity uses `age-keygen` and defaults to `~/.shine/age/identity.txt`.

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

`--touch-id` is macOS-only. Decryption requires the local Secure Enclave and prompts for Touch ID or
the system passcode; copying the identity file to another machine is normally insufficient to
decrypt. Cancel unexpected authorization prompts and inspect the command that caused them. For the
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

The legacy `age_identity` path and the additional `age_identities` list are merged in order and
deduplicated. This lets a normal or Secure Enclave identity coexist with hardware-plugin stubs
without copying either file.

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

```bash
shine state migrate --dry-run
shine state migrate
```

The migration updates configured `age1se...` values to equivalent `age1tag...` values without
reading identities or decrypting secrets. It checks the global config, current project config, and
nearest workspace together; dry-run reports per-file counts, and applying preserves array order,
comments, and duplicates. All legacy recipients are validated before any file is written. Reseal
afterward to write new ciphertext using the native tagged recipient. Existing ciphertext remains
unchanged and decryptable as before.

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

[`age-plugin-phone`](https://github.com/biulight/age-plugin-phone) is currently an owner-only
technical preview for synthetic or disposable data, not real or production secrets. Its Windows
Alpha requires a Windows 11 x64 client, TPM 2.0, Microsoft Platform Crypto Provider, and a
capability-qualified Android StrongBox phone. Follow the project's
[`Windows Alpha quick start`](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-alpha-quickstart.md)
for artifact verification, pairing, transport, recovery drills, and cleanup.

Shine also opens the experimental macOS pairing flow. Install a desktop plugin build containing
the macOS implementation and a matching Android StrongBox app, following the plugin
[macOS source quick start](https://github.com/biulight/age-plugin-phone/blob/main/docs/macos-quickstart.md).
It requires real Secure Enclave support in a logged-in user session; Intel/T2 and other hardware
or OS versions are not generally verified. A compilation deployment floor is not a tested minimum
supported macOS version. Use only disposable data and keep an independently verified recovery
recipient. The plugin performs the hardware checks.

For example, on a Mac with an authorized Android ADB device:

```sh
shine env secret identity init --phone --label "Work Mac" --transport adb --adb-serial SERIAL
```

After installing the matching desktop plugin and Android application, start its transactional
pairing through Shine:

```powershell
shine env secret identity init --phone --label "NUC WiFi Pair" --transport auto
```

Before running the command, open the phone's explicit **Pair · Wi-Fi** action if you want to pair
over the local network. On Windows and macOS, `auto` performs one bounded Wi-Fi discovery first. Exactly one
matching foreground phone listener selects Wi-Fi; if no listener responds, setup selects Developer
USB/ADB on Windows or QR on macOS before creating the pairing offer. QR requires a supported
camera and the plugin's scan flow. Ambiguous discovery or a local discovery error fails
closed, and an attempt never switches transport after protocol work begins. `auto` is the default,
so omitting `--transport auto` keeps the same policy.

Developer USB uses the opposite order. Start the desktop command first; after the plugin has
selected ADB and is waiting for the phone connection, choose **Pair · USB** on the phone. With
`--transport adb`, ADB is selected directly after preflight. The phone makes one immediate
connection attempt, so choosing **Pair · USB** before the desktop has armed its reverse rule reports
`usb_transport_failed`.

The pairing label defaults to the computer name on Windows and macOS, or `Shine desktop` if no
valid name is available. Explicit labels must be nonblank and at most 64 UTF-8 bytes. Override it, pin Developer USB or QR, or
select one of multiple ADB devices explicitly when needed:

```powershell
shine env secret identity init --phone --label "Work laptop"
shine env secret identity init --phone --transport adb
shine env secret identity init --phone --transport qr
shine env secret identity init --phone --adb-serial SERIAL
```

Shine leaves all pairing, hardware keys, replay, locator, interruption, and cleanup state under the plugin's
ownership. After successful fingerprint confirmation, it adds only the public identity-stub path to
the current user's global `age_identities`. If the active project explicitly overrides
`age_identity` or `age_identities`, the command stops before pairing instead of creating an identity
that the project would ignore. A manual configuration has this shape (use the actual absolute
identity path returned by the plugin on your platform):

```toml
age_identities = ["/absolute/path/returned/by/plugin/identity.txt"]
```

The shortcut does not change `secret_backend` and does not add recipients. Interrupted plugin setup
must be handled with `age-plugin-phone setup --resume` or `age-plugin-phone setup --cleanup` as
described by the plugin; do not start a second pairing as recovery.

Put the printed `age1tag...` recipient and an independently verified recovery recipient in the
project's commit-ready `shine.workspace.toml`:

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1tag...", "age1..."]
```

Shine requests `--recipient-type tag` by default. This requires age 1.3+ before pairing and a
desktop plugin and phone app that both support tagged recipients. This integration depends on that
plugin capability; older preview builds may reject the option. Shine does not retry setup or fall
back to another recipient type. Once supported, `age1tag...` encryption needs only age 1.3+ on the
encrypting computer, with no phone plugin or phone prompt. Pairing and phone decryption still need
the plugin and matching phone app.

To retain the plugin's phone recipient format, explicitly select:

```powershell
shine env secret identity init --phone --recipient-type phone
```

`age1phone...` encryption still requires `age-plugin-phone` on every encrypting computer.
The plugin's own default can remain `phone`; Shine explicitly requests its chosen type.
Unlike phone v2's private recipient selection, a tagged recipient lets someone who knows the
recipient test whether a ciphertext targets it.

For an existing pairing, first upgrade and verify the desktop plugin and phone app's tag support,
then export its public tag recipient without pairing again:

```powershell
age-plugin-phone recipients -i <IDENTITY_STUB> --recipient-type tag
```

Replace the corresponding recipient in global/project `age_recipients` or workspace
`[env.encryption].age_recipients`, retaining the independently verified recovery recipient, then
reseal. `shine state migrate` does not convert phone recipients. Export does not rewrite the stub
or existing ciphertext; `identity list` continues to show the recipient recorded in the stub.
Resealing an existing payload first decrypts it and may require phone authorization; see
[Add recipients to existing workspace secrets](#add-recipients-to-existing-workspace-secrets).
Changing recipients does not revoke access to historical ciphertext. For later
Wi-Fi-first decrypts with an `auto` pairing, enable **Wi-Fi auto-listen** and keep the phone app in
the foreground. The plugin discovers the matching listener before creating the unwrap request and
otherwise selects Developer USB/ADB on Windows or QR on macOS; it does not race routes or retry in flight.
Decrypting a phone-backed secret, including through `shine env run`, invokes the standard age
plugin and must require a fresh strong biometric authorization for each file-key unwrap. Developer
USB and Wi-Fi plugin guidance is quiet by default; set `AGE_PLUGIN_PHONE_MESSAGES=1` to opt into it.
Explicit QR requests remain visible because the phone must scan them. A successful
`shine env secret decrypt` writes only the decrypted value, suppresses the age client's own waiting
diagnostic, and does not append a line ending. A shell theme may still place its next prompt on a
fresh line. Never make the experimental phone recipient the only recipient for retained data; the
recovery path must not depend on the same phone StrongBox keys, desktop TPM/Secure Enclave keys, or plugin state.

If AI agents participate in development, read
[Protect environment secrets when using AI agents](./agent-secret-safety.md) first to understand
the boundaries around identity files, Touch ID, phone authorization prompts, and command execution.

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
# age_recipients = ["age1tag1qexample...", "age1qteammate..."]
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

For single-backend policy and sources, usable recipients let `env run` store an encrypted,
mode-specific cache with the selected backend in the system cache directory. It rebuilds automatically when workspace content, source content, or file
order changes.

Ignore personal overrides:

```gitignore
.env.local.shine.toml
.env.*.local.shine.toml
```

Never commit a source containing unsealed strings. Before committing, inspect `[secret]` and confirm
every sealed entry is `true`. See the [configuration reference](../reference/configuration.md) for
file formats and precedence.

## Share a payload across GPG and age members

Upgrade all readers, including the local SSH broker machine, before conversion.
Existing untagged GPG and `age:` ciphertext stay readable; ordinary reads never migrate
files. Explicitly configure the workspace (replace the public placeholders):

```toml
[env.encryption]
backend = "hybrid"
gpg_recipients = ["<full 40-hex primary fingerprint>"]
age_recipients = ["age1..."]
```

Run `shine env secret seal --workspace shine.workspace.toml`. Reading old ciphertext
requires its own identity; resealing requires both encryption tools and complete public
lists. Either group independently reads the same data ciphertext. Hybrid GPG wrapping
excludes local option files and implicit extra recipients; groups and fuzzy user IDs
are rejected before existing secrets are decrypted.

For hybrid policy or a hybrid source consumed by the current mode, `env run` stores
an encrypted compilation cache using the locally selected GPG or age backend. Only
that backend is needed to create the cache. Recipients come from the workspace's
corresponding list, never global recipients. An absent list skips caching; a cache
write failure warns but does not prevent running successfully compiled values.

A cache hit decrypts the merged environment once. There is no TTL or saved unlock
state: backend authorization still applies on every read. Workspace/source content,
source order, selected backend, or recipients changing invalidates the cache. GPG
and age caches are separate; legacy caches cannot substitute for hybrid sources.
Malformed hybrid envelopes fail before cache decryption. Once cache decryption
starts, failure or cancellation stops the command without retrying from sources.
Export does not use this cache: it omits secrets unless `--include-secrets` is set,
and dry-run never decrypts secrets.

For a personal project preference, create `shine.config.local.toml` alongside the
selected `shine.workspace.toml` (also when using `--workspace`):

```toml
hybrid_decrypt_backend = "age"
```

Add `/shine.config.local.toml` to that project's `.gitignore`. Local `env run`,
`env secret seal`, and `env workspace export` use this preference. An absent file
or field inherits global `config.toml`; without either preference, existing
capability checks and interactive selection apply. Invalid configuration fails
explicitly. The file currently accepts only `hybrid_decrypt_backend = "gpg" | "age"`.
It is trusted local project configuration, so a program able to edit it can change
the chosen authorization path. SSH broker requests do not load or transmit this file;
the decrypting broker machine retains its own configured choice and release checks.

Sealing locks out cooperating Shine sealers and rechecks captured workspace and source
bytes before replacement. Edits during hardware approval abort the current file;
earlier completed files remain completed and remaining files are not processed. Do
not edit during sealing: editors ignoring the lock can race in the final comparison
to replacement interval. Failed sealing may leave pending plaintext entries; Shine
does not claim they were cleared. `.shine-seal.lock` files may remain, contain no
secrets, and must not be deleted while a seal is active.
