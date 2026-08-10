---
title: SSH sessions, secret brokering, and file transfer
sidebar_position: 6
---

# SSH sessions, secret brokering, and file transfer

`shine ssh` opens a normal SSH session with optional selected environment forwarding, on-demand local
secret decryption for remote commands, and a file-transfer channel between local and remote hosts.
See [Manage environment variables and secrets](./environment.md#provide-variables-and-secrets-to-remote-commands)
for local value management.

## Open a session

```bash
shine ssh user@example.com
shine ssh -p 2222 user@example.com
shine ssh user@example.com uname -a
```

Shine forwards the supplied SSH arguments to the system `ssh`. In a remote shell it sets variables
needed by the session; transfer commands must run inside that shell.

Default `posix` mode requires a macOS or Linux remote capable of running a compatible version of
`shine local`. Windows can initiate such a session as the local host. For a Windows remote, use
`--remote-shell windows`; that mode forwards environment values but provides no file transfer.

## Forward selected environment values

Place forwarding options before the SSH target:

```bash
shine ssh --with API_URL dev
shine ssh --with LOCAL_NAME=REMOTE_NAME dev 'printenv REMOTE_NAME'
shine ssh --with-secret API_TOKEN dev
```

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

All options appear before the SSH target. Except inspect and enroll, they require `--secret-broker`.

| Option | Purpose | Repetition and combinations |
| --- | --- | --- |
| `--secret-broker` | Enable on-demand requests for this session | Combines with allow-list, policy, and trusted-session options |
| `--allow-secret KEY[=ALIAS]` | Allow a direct request for local encrypted `KEY_SECRET` | Repeatable; requires local confirmation for every request |
| `--secret-broker-policy FILE` | Load an additional local-only policy file | Repeatable; owner, mode, and non-symlink checks still apply |
| `--trust-remote-session` | Skip per-request confirmation for an exact workspace-policy match | Never affects direct `--allow-secret` requests |
| `--secret-broker-inspect` | Inspect remote workspace, source digests, command, and policy comparison | Cannot combine with broker or enroll; releases nothing and writes nothing |
| `--secret-broker-enroll` | Create a local policy from remote-reported metadata | Requires `--trust-remote-metadata`; releases nothing and runs no target command |
| `--trust-remote-metadata` | Explicitly trust this remote report as policy source | Valid only with enroll |
| `--update-policy NAME` | Update one existing policy from a trusted remote report | Enroll only; target, mode, and complete command must satisfy update constraints |

These options cannot be combined with `--remote-shell windows`, which lacks the required POSIX
control channel.

For a temporary command, allow an encrypted local configuration key:

```bash
# Local: every request requires local confirmation.
shine ssh --secret-broker --allow-secret API_TOKEN dev

# Remote: request API_TOKEN only for this child.
shine env run --no-workspace --secret-broker --secret API_TOKEN -- bun run build
```

The two key mappings must match and both accept `KEY=ALIAS`. Direct requests read only local
`KEY_SECRET`, never fall back to plaintext `KEY`, and always require local confirmation even with
`--trust-remote-session`. Pass the base key, not the `_SECRET` suffix. A session uses the encrypted
snapshot frozen when it connected, even if local configuration changes later.

For a fixed project, register an exact policy from a **trusted local checkout**. Policies record the
SSH target, workspace and source digests, mode, complete argv, and releasable keys—never plaintext:

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

`--release DEPLOY_TOKEN` is the allow-list of workspace secrets the local side may release to the
target child. It does not send a secret during policy creation or choose a remote variable by itself.
Shine first verifies every recorded digest, mode, and argument, then decrypts locally and injects the
value briefly into exactly `bun run build`. Repeat `--release` for multiple keys. Other `[secret]`
keys in the same source remain unavailable.

To select every secret declared by the current mode, use `--release-all-declared`. It expands and
records the complete list when the description or policy is created; it is not a runtime wildcard.
Future keys do not become authorized automatically. Choose either repeated `--release` or
`--release-all-declared`, and choose at least one:

```bash
shine env broker policy add \
  --name dev-api-build \
  --ssh-target dev \
  --workspace ~/src/acme-api/shine.workspace.toml \
  --mode development \
  --release-all-declared \
  -- bun run build
```

Run the exact command from the remote project:

```bash
shine env run --mode development --secret-broker -- bun run build
```

Local confirmation remains the default. Add `--trust-remote-session` to the local SSH command only
after accepting the risk of trusting that remote session and same-account processes. It automatically
approves only exact workspace-policy matches.

Load a protected temporary or team policy file without writing it into the default
`~/.shine/ssh-secret-broker.toml`:

```bash
shine ssh --secret-broker \
  --secret-broker-policy ~/.config/shine/staging-broker.toml \
  staging
```

Additional and default policies are merged. A request must match exactly one; zero or multiple
matches are rejected.

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

Without a local checkout, open an inspect session and run `describe` remotely. It displays workspace
and source digests, mode, complete argv, and release keys and compares them with local policies:

```bash
# Local
shine ssh --secret-broker-inspect dev

# Remote
cd /srv/acme-api
shine env broker describe --mode development \
  --release-all-declared -- bun run build
```

Inspect releases nothing and changes no policy. `--release` on `describe` identifies candidate policy
keys only. If the host and metadata have been verified out of band, enroll with an explicit trust
acknowledgment:

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

The policy must belong to the current SSH target, and any constrained remote workspace path must
match. Confirmation shows the full TOML diff. Approval replaces only the matched allow entry and
refreshes digests while preserving the policy name, project, remote-path constraint, and other
entries. A concurrent modification after preview makes the write fail. This option does not reduce
the trust risk of remote metadata.

## Connect to a Windows remote

Select the PowerShell wrapper before the target:

```bash
shine ssh --remote-shell windows --with-secret GH_TOKEN windows-host
shine ssh --remote-shell windows --with API_URL windows-host Get-ChildItem Env:API_URL
```

Shine prefers PowerShell 7 (`pwsh.exe`) and falls back to Windows PowerShell 5.1
(`powershell.exe`). It safely injects selected environment variables and the local terminal theme.
Interactive sessions load the normal selected-PowerShell profile so Shine-managed `PATH` and command
wrappers work; explicit remote commands run with no profile.

Windows mode creates no transfer tunnel, so `shine local download`, `upload`, and `status` are
unavailable. Use system `scp`, `sftp`, or another transfer tool.

## Download from remote to local

Inside a remote shell opened by `shine ssh`:

```bash
shine local download ./logs/app.log
shine local download ./logs/app.log ./downloaded/app.log --dry-run
shine local download ./logs/app.log ./downloaded/app.log --force
shine local download ./logs/app.log --scp
shine local download ./dist ./dist-copy
```

The remote side resolves the source; the local side resolves the destination. Without a destination,
the item is placed in the local directory from which `shine ssh` started, using the source name.

The local side starts system `rsync` or `scp`, preferring rsync and falling back to scp. Both ends need
SSH, and directory transfer requires a common tool. Existing files are not overwritten by default;
for an existing directory, `--force` merges into it. Use `--scp` to skip rsync probing.

## Upload from local to remote

Still inside the remote shell:

```bash
shine local upload ./release.tar.gz /tmp/release.tar.gz --dry-run
shine local upload ./release.tar.gz /tmp/release.tar.gz --force
shine local upload ./site /tmp/site
```

The local side resolves the source and the remote side resolves the destination. Without a
destination, Shine places it in the remote current directory using the source name. Directory upload
refuses file-to-directory or directory-to-file replacement. Preview resolved paths and overwrite
behavior with `--dry-run`.

## Inspect connection state

```bash
shine local status
```

The output includes session ID, reachability, protocol version, and the local default directory. A
shell not entered through `shine ssh` reports the missing session variables.
