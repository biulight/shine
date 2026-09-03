---
title: Protect environment secrets when using AI agents
sidebar_position: 7
---

# Protect environment secrets when using AI agents

When AI agents such as Claude Code or Codex participate in development, secret safety involves more
than excluding `.env` from Git. An agent may read workspace files, run commands, and inspect output.
Long-lived plaintext secrets in a project can easily reach logs, patches, context, or remote services.

Shine's `env secret seal`, `env run`, and `age` backend reduce this spread by storing ciphertext in
the repository and decrypting values only for the child process that needs them. They are not a
sandbox and do not replace operating-system isolation. Understand the boundary between identity
files, hardware authorization, and agent permissions before use.

## What Shine environment protection covers

`shine env secret seal` seals pending secrets in workspace environment files into an encrypted
payload. The repository then contains ciphertext rather than plaintext tokens, passwords, or API
keys.

```bash
shine env secret seal
```

`shine env run` merges environment files, decrypts secrets, and provides the result only to the
started child process:

```bash
shine env run --mode development -- bun run build
```

This primarily reduces three risks:

- plaintext secrets remaining in project files;
- exporting a secret into an entire shell session merely to run one task;
- an AI agent reading, copying, or committing a plaintext `.env` while editing code.

An agent permitted to run a command that reads environment variables can still see secrets visible
to that command. The boundary is on-demand injection, not protection from an untrusted child.

For an occasional credentialed operation, run the command yourself with one-time `env run --with`
injection. If a CLI repeatedly needs the same fixed credential variable, a transparent command
proxy can keep the normal invocation:

```bash
shine env proxy install gh --with GH_TOKEN
gh pr list
```

An agent allowed to run `gh` can still use the injected token and inspect anything the command
reveals. The proxy reduces persistent plaintext and shell-wide exports; it does not make the target
command trusted. See [choose one-time injection or a transparent wrapper](./environment.md#choose-one-time-injection-or-a-transparent-wrapper)
for setup, enable/disable behavior, and the Cargo example.

## An age identity is decryption authority

With the `age` backend, `age_recipients = ["age1..."]` identifies who can decrypt. A personal default
may live in `~/.shine/config.toml`; a project team's shared recipient list belongs in
`[env.encryption]` in the commit-ready `shine.workspace.toml`.

```toml
secret_backend = "age"
age_recipients = ["age1se1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
```

`~/.shine/age/identity.txt` is the private decryption identity. Never commit or share it, and do not
put it in a workspace an agent can freely read.

```bash
shine env secret identity init
shine env secret identity list
```

On Unix and macOS, Shine gives generated identity files mode `0600`, readable and writable only by
the current user. This blocks other local users, but not an agent, script, or process running as the
same user with permission to read that path. A normal age identity protects repository and transport
ciphertext, not the entire local runtime.

## What Touch ID improves

On macOS, create a Secure Enclave and Touch ID identity:

```bash
shine env secret identity init --touch-id
```

`age-plugin-se` generates the identity. Decryption requires the local Secure Enclave and a Touch ID
or system PIN authorization. Copying its identity file to another machine is normally insufficient
to decrypt.

This makes an identity harder to abuse offline, requires local user authorization, and prevents an
agent from decrypting elsewhere with only the file. It is not absolute isolation: an agent able to
run a local decrypt command can still trigger the system prompt. Cancel unexpected Touch ID or PIN
prompts and inspect the command that caused them.

## Collaborating from Windows

Windows members can use a normal age identity in a multi-recipient setup:

```bash
shine env secret identity init
shine env secret identity list
```

Add its `age1...` recipient alongside macOS Touch ID recipients so one ciphertext supports both.

For teams exploring fresh hardware authorization on every decrypt, Shine's author also develops the
standalone [`age-plugin-phone`](https://github.com/biulight/age-plugin-phone) project. It grew out of
direct Windows hardware-identity work in Shine that reached a platform capability limit. The Shine
proof of concept found that Windows Hello's Passport provider could perform only the legacy RSA
PKCS#1 v1.5 unwrap; RSA OAEP-SHA256, P-256 ECDH, and the tested WebAuthn PRF path were unavailable.
Rather than ship that legacy construction or introduce a Shine-specific ciphertext format, the
author moved the work behind the standard age plugin protocol and into a separately reviewable
project.

The current design keeps the long-term age decryption key in Android StrongBox and requires a fresh
strong biometric authorization on the phone for each file-key unwrap. The Windows TPM holds only
two role-separated, non-exportable P-256 keys for authenticating the paired desktop and privately
selecting its recipient stanza. It never receives the phone's long-term private key, and there is
no DPAPI, software-identity, password, or cached-authorization fallback. Shine continues to use the
standard `age` CLI, `identity-v1`, and `recipient-v1`; the plugin adds no Shine dependency or custom
ciphertext.

This avoids relying on Windows Hello for the missing cryptographic operations, but it does not
remove the current platform prerequisites. Version `0.1.0-alpha.1` is an owner-only technical
preview requiring a Windows 11 x64 client, TPM 2.0, Microsoft Platform Crypto Provider, and a
capability-qualified Android StrongBox phone. Developer USB/ADB is the normal Windows transport;
QR is an explicit fallback, not an automatic downgrade. Protocol v2, public signing, multi-device
coverage, and the complete lifecycle matrix are not finished. Use it only with synthetic or
disposable data, never real or production secrets. Follow the project's
[`Windows Alpha quick start`](https://github.com/biulight/age-plugin-phone/blob/main/docs/windows-alpha-quickstart.md)
for the exact artifact, pairing, transport, recovery, and cleanup procedure.

The plugin uses only Shine's existing age identity and recipient settings. See
[experiment with phone authorization on Windows](./environment.md#experiment-with-phone-authorization-on-windows)
for the exact machine and workspace configuration. The identity stub contains public pairing
material, not the phone's long-term private key.

On the supported preview platform, `shine env secret identity init --phone` launches the plugin's
own transactional setup and records only its public stub path in global `age_identities`. It does
not manage private plugin state, switch the default backend, or add a phone-only recipient set.

The recovery path must not depend on the same phone StrongBox keys, Windows TPM keys, or plugin
state. Never make the experimental phone recipient the only recipient for retained data. A normal
age identity remains suitable for ordinary team development when its file and user-directory
permissions are protected. For stable hardware-backed protection on Windows, prefer an
organization-approved YubiKey/PIV or GPG with YubiKey workflow.

## Choose a secret backend

A rough ordering by isolation strength is:

1. GPG with YubiKey or another hardware smart card;
2. age with Secure Enclave and Touch ID;
3. a normal age identity file;
4. plaintext secrets.

Age with Touch ID is often more convenient for team development and staging. For valuable production
secrets, long-lived credentials, or strong hardware-isolation requirements, prefer GPG with YubiKey
or an organization-approved hardware-backed design.

The experimental phone design aims for hardware-backed, authorization-per-use isolation, but it is
not included in this stable-backend ordering until its protocol and release gates are complete.

## Permissions for AI agents

Treat an agent as a capable local collaborator, not as an inherently trusted security boundary:

- Never add `~/.shine/age/identity.txt`, GPG private keys, or cloud credential files to a workspace.
- Do not let an agent keep an interactive shell with high-privilege secrets for long periods.
- Prefer letting the agent edit code and running `shine env run` yourself in a trusted terminal.
- Use a normal age identity for low-risk development and Touch ID or YubiKey for sensitive work;
  limit `age-plugin-phone` to its documented synthetic-data preview.
- Cancel unexpected Touch ID, phone biometric, PIN, or YubiKey-touch prompts.

Removing a recipient after an identity leaks, a device becomes untrusted, or a member leaves does not
revoke access to historical ciphertext. Reseal or re-encrypt it and rotate the real upstream token
when necessary:

```bash
shine env secret seal
```

Do not commit environment files containing unsealed strings. Add personal override files to
`.gitignore`, and keep only sealed ciphertext in shared files.
