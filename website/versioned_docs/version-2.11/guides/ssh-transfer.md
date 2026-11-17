---
title: SSH sessions, secret brokering, and file transfer
sidebar_position: 6
---

# SSH sessions, secret brokering, and file transfer

`shine ssh` opens a normal SSH session with optional selected environment forwarding, on-demand local
secret decryption for remote commands, and a file-transfer channel between local and remote hosts.
See [Manage environment variables and secrets](./environment.md#provide-variables-and-secrets-to-remote-commands)
for local value management.

## Version compatibility {#version-compatibility}

When a Shine channel reports a protocol version mismatch, upgrade the older local or remote
Shine and reconnect. Coordinate both peers before upgrading automated workflows.

## Open a session

Choose one connection example:

| Operation | Command |
| --- | --- |
| Open an interactive remote shell | `shine ssh user@example.com` |
| Use a custom SSH port | `shine ssh -p 2222 user@example.com` |
| Connect to an IPv6 address | `shine ssh user@2001:db8::1` |
| Run one remote command | `shine ssh user@example.com uname -a` |

Shine forwards the supplied SSH arguments to the system `ssh`. Run file-transfer commands inside
the remote shell opened by Shine.

Default `posix` mode requires a macOS or Linux remote capable of running a compatible version of
`shine local`. Windows can initiate such a session as the local host. For a Windows remote, use
`--remote-shell windows`; that mode forwards environment values but provides no file transfer.

If a transfer is interrupted, files already copied are not rolled back. Check both locations
before retrying.

## Forward selected environment values

Place forwarding options before the SSH target:

| Operation | Command |
| --- | --- |
| Forward a plain variable | `shine ssh --with API_URL dev` |
| Rename the forwarded variable | `shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'` |
| Forward a decrypted secret | `shine ssh --with-secret API_TOKEN dev` |

`--with` reads only the exact plaintext `[env]` key and never decrypts `KEY_SECRET` automatically.
Use `--with-secret KEY[=ALIAS]` explicitly for decryption. Values exist only in the remote process
environment, not remote configuration files, although remote shell startup files may override them.

Forwarding a secret makes plaintext available to the remote host. Privileged local or remote users
and other same-account processes may also observe process arguments or environments. Forward only
necessary keys to trusted hosts and never put tokens directly on a command line.

## Provide secrets to remote commands on demand

Use the Secret Broker when the remote project has sealed workspace ciphertext but the private key or
YubiKey remains local. Plaintext never enters the login shell: a specifically approved remote child
requests local decryption and receives it only for that process.

The broker supports POSIX remotes only. It cannot protect a remote controlled by an administrator,
root, or malicious same-account process, and the target program can read plaintext while running.
Prefer short-lived credentials or workload identity for production services that can support them.

### Options

Put Shine options before the SSH target. Use `--secret-broker` for a session that can release
secrets. Inspection (`--secret-broker-inspect`) and enrollment (`--secret-broker-enroll`) use
separate sessions and cannot be combined with it or with each other. All broker modes require a
POSIX remote; they cannot be combined with `--remote-shell windows`.

The examples below cover each workflow. For the complete option list, run `shine ssh --help`.

### Use a local secret for one command

For a temporary command, allow an encrypted local configuration key:

```bash
# Local: every request requires local confirmation.
shine ssh --secret-broker --allow-secret API_TOKEN dev

# Remote: request API_TOKEN only for this child.
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

The two key mappings must match and both accept `KEY=ALIAS`. Direct requests read only local
`KEY_SECRET`, never fall back to plaintext `KEY`, and always require local confirmation even with
`--trust-remote-session`. Pass the base key, not the `_SECRET` suffix. Reconnect after changing a
local secret; existing sessions continue using the value selected when they connected.

### Authorize a project command

For a fixed project, create a policy from a **trusted local checkout**. It authorizes the selected
secrets for one SSH target, workspace version, mode, and exact command. Policies do not save plaintext:

```bash
shine env broker policy add \
  --name dev-api-build \
  --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development \
  --release DEPLOY_TOKEN \
  -- bun run build

shine ssh --secret-broker dev
```

`--release DEPLOY_TOKEN` allows that workspace secret to reach the approved command; creating the
policy does not send it. Repeat `--release` for multiple keys. Unlisted secrets remain unavailable.

Instead of repeated `--release`, use `--release-all-declared` to authorize all secrets declared by
the current mode. Choose one form; they cannot be combined. This selection covers only the current
keys, not secrets added later.

Run the exact command from the remote project:

```bash
shine env run --mode development --secret-broker -- bun run build
```

Workspace values follow source order, just as in local `env run`. A later plain value overrides an
earlier secret; the shadowed secret is not sent to the remote. A later secret declaration hides an
earlier plain value even when that secret is not released.

Local confirmation remains the default. Add `--trust-remote-session` to the local SSH command only
after accepting the risk of trusting that remote session and same-account processes. It automatically
approves only exact workspace-policy matches.

### Load or update policies

To use a temporary or team policy file in addition to the default `~/.shine/ssh-secret-broker.toml`:

```bash
shine ssh --secret-broker \
  --secret-broker-policy ~/.config/shine/staging-broker.toml \
  staging
```

Keep policy files local and private. On Unix they must belong to you and have `0600` permissions;
symlinks are rejected. You can repeat `--secret-broker-policy` for additional files. A request must
match exactly one policy across all files; no match or multiple matches are rejected.

Review a policy change before updating it. Arguments are exact, not wildcarded, so changing the
command, mode, or source requires review:

```bash
shine env broker policy diff dev-api-build \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development --release DEPLOY_TOKEN -- bun run build
shine env broker policy update \
  --name dev-api-build --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development --release DEPLOY_TOKEN -- bun run build
```

Adding an existing policy name fails without replacing it. To update it, use `--name` and supply
the policy parameters again, including any project or remote-workspace constraints; see
`shine env broker policy update --help`. Changed policies still require review and confirmation.

Use `shine env broker policy list` to browse policies and `shine env broker policy info <NAME>`
to inspect one. If a policy changes while you are reviewing it, inspect the new version and retry.

### Review a project available only remotely

Without a trusted local checkout, inspect the remote project before creating a policy. Open an
inspect session locally and run `describe` in the remote project. Review its workspace, source
files, mode, command, and proposed secret access against your local policies:

```bash
# Local
shine ssh --secret-broker-inspect dev

# Remote
cd /srv/acme-api
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

Inspect does not release secrets or change policies. `--release` on `describe` only identifies
proposed access. If you have independently verified the remote host and project, reconnect in
enrollment mode to create the policy:

```bash
# Local
shine ssh --secret-broker-enroll --trust-remote-metadata dev

# Remote: sends only a description.
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

Enroll shows the candidate locally and writes only after local confirmation. It never silently
overwrites a same-name or overlapping policy; prefer `policy diff` and `policy update` from a trusted
checkout.

When only the remote copy exists and you accept it as the source, update an existing policy explicitly:

```bash
# Local
shine ssh --secret-broker-enroll --trust-remote-metadata \
  --update-policy dev-api-build dev

# Remote: mode and command must match exactly one allow entry.
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

The policy must match the current SSH target and any constrained workspace path. Review the
proposed changes locally before confirming. Updating from remote metadata still requires trusting
that remote; prefer a verified local checkout when available.

### Hybrid secrets through the broker

For hybrid secrets, set `hybrid_decrypt_backend` in the local broker machine's global configuration.
The remote cannot choose the decryption backend or identity. Local approval is still required
unless an exact policy match has been authorized for the session; cancellation does not try another
backend. Only approved plaintext values reach the remote, never decryption keys. After resealing
or changing recipients, review and update the affected policies.

## Authorize secret files locally {#file-secret-broker}

Use file brokering to decrypt files locally for an approved operation on a POSIX remote. Upgrade
both Shine installations before using it. A version mismatch blocks release, and existing
environment-variable policies do not authorize files.

For an App with automatic secret installation, create an exact local policy from your trusted overlay:

```bash
shine env broker policy add --name service-install --ssh-target server \
  --workspace ~/overlay/app/example/shine.workspace.toml \
  --remote-workspace /srv/example-service/shine.workspace.toml \
  --mode production --file-group credentials --app example --operation install \
  --migrate-file-policy
shine ssh --secret-broker server
# On the server:
shine app install example
```

Create a separate policy with `--operation upgrade` for upgrades. Before using
`--migrate-file-policy`, privately back up `~/.shine/ssh-secret-broker.toml`: this option upgrades
the policy format while preserving existing environment policies. Downgrading Shine does not undo
that migration.

Review the approved host, operation, files, destinations, and permissions. Requests can use only
the authorized files; new or changed sources require a reviewed policy update. Use the same file
arguments with `policy update` or `policy diff`.

Without a trusted local checkout, use the existing separate inspect/enroll sessions:

```bash
shine ssh --secret-broker-inspect server
# On the server, inspect without decrypting:
shine env broker describe --app example --operation install \
  --mode production --file-group credentials
```

Use `shine ssh --secret-broker-enroll --trust-remote-metadata server` only when you trust the
remote description. Run the same describe command and confirm locally; reconnect with
`--secret-broker` afterward. `--update-policy NAME` selects an existing file policy to replace.
Inspect and enrollment sessions never return plaintext.

For manual file export, omit `--app`, authorize the installed workspace with a separate export
policy, and run on the remote:

```bash
shine env workspace export --format files --workspace /srv/example-service/shine.workspace.toml \
  --mode production --file-group credentials --secret-broker
```

Local confirmation is the default. `--trust-remote-session` may suppress it only for an exact
policy match; the private key stays local, but authorized plaintext is transferred to the remote.
Manual export refuses to overwrite existing targets. It does not make the exported files
App-managed; you remain responsible for their permissions and cleanup.

## Connect to a Windows remote

Select the PowerShell wrapper before the target:

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN windows-host
shine ssh --remote-shell windows --with API_URL windows-host Get-ChildItem Env:API_URL
```

Shine uses PowerShell 7 when available, otherwise Windows PowerShell 5.1. Interactive sessions load
your normal PowerShell profile; explicit remote commands do not. If a command depends on your
profile settings, account for that when running it remotely.

Windows remote mode does not support `shine local download`, `upload`, or `status`. Use system
`scp`, `sftp`, or another transfer tool.

## Download from remote to local

Run these commands inside the remote shell opened by `shine ssh`. Preview the destination before
allowing replacement; SCP may overwrite existing files even without `--force`.

| Operation | Command |
| --- | --- |
| Download to the local starting directory | `shine local download ./logs/app.log` |
| Preview destination | `shine local download ./logs/app.log ./downloaded/app.log --dry-run` |
| Allow replacement at the destination | `shine local download ./logs/app.log ./downloaded/app.log --force` |
| Use SCP directly | `shine local download ./logs/app.log --scp` |
| Download a directory | `shine local download ./dist ./dist-copy` |

The remote side resolves the source; the local side resolves the destination. Without a destination,
the item is placed in the local directory from which `shine ssh` started, using the source name.

Transfers prefer `rsync` and fall back to `scp`. Both ends need SSH and a common transfer tool
for directories. Rsync skips existing files unless `--force`
is supplied; for an existing directory, `--force` merges into it. Use `--scp` to skip rsync probing.
IPv6 literal SSH destinations use SCP for compatibility with system rsync. SCP may overwrite existing
files even without `--force`, and Shine prints a warning. Its `--dry-run` shows resolved paths without
copying files.
With rsync, quote paths containing spaces and wildcard patterns in the command you enter:

```bash
shine local download './build output/*.log'
shine local upload './release notes.txt' '/tmp/release files'
```

Spaces and shell punctuation remain part of the remote path; `*`, `?`, and bracket patterns still
select matching remote files.

## Upload from local to remote

Run uploads inside the same remote shell. Preview first, and use `--force` only when replacement
is intended.

| Operation | Command |
| --- | --- |
| Preview upload | `shine local upload ./release.tar.gz /tmp/release.tar.gz --dry-run` |
| Allow replacement at the destination | `shine local upload ./release.tar.gz /tmp/release.tar.gz --force` |
| Upload a directory | `shine local upload ./site /tmp/site` |

The local side resolves the source and the remote side resolves the destination. Without a
destination, Shine places it in the remote current directory using the source name. Directory upload
refuses file-to-directory or directory-to-file replacement. Preview resolved paths and overwrite
behavior with `--dry-run`.

Local transfer paths must be valid UTF-8. Local path hints expand only a leading `~`;
`$VAR` and `${VAR}` remain literal filename text.

## Inspect connection state

```bash
shine local status
```

Use the result to check whether the session is reachable and confirm the local default directory.
If session variables are missing, reconnect with `shine ssh`. Connection or authentication failures
require checking the SSH session before retrying a transfer.
