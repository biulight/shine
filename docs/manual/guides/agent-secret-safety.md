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

## Prepare a Shine workspace

A Shine workspace consists of `shine.workspace.toml` at the project root and the `*.shine.toml`
environment sources it references. The workspace file declares modes such as `development`, the
source merge order, and shared encryption recipients. The sources hold ordinary configuration and
secrets. These are project environment files; creating them does not isolate an agent's access.

If the project already has `.env` files, run the following yourself in a trusted terminal at the
project root. Replace `DATABASE_URL` with an actual sensitive key; repeat `--secret` for each one:

```bash
shine env workspace init --from-dotenv --secret DATABASE_URL --dry-run
shine env workspace init --from-dotenv --secret DATABASE_URL
```

This generates `shine.workspace.toml` and corresponding sources, such as `.env.shine.toml` from
`.env`. Initialization only imports values: keys selected with `--secret` are still plaintext in
`[secret]` until sealed, and unselected keys remain plaintext in `[plain]`. Review the classification
before continuing. See [initialize a workspace from dotenv](./environment.md#initialize-a-workspace-from-dotenv)
for supported inputs and mode selection. If there are no `.env` files, follow
[layered project environments](./environment.md#use-layered-project-environments) to create the
workspace and sources manually.

Next, follow [use age identities](./environment.md#use-age-identities) to install the dependencies,
set up your local identity, and configure the backend and recipients. Then seal the sources and
verify your project command through Shine:

```bash
shine env secret seal
shine env run --mode development -- bun run build
```

Use a mode declared in your workspace and replace `bun run build` with your project's command.
Neither initialization nor sealing changes the original `.env` files. Once the project works through
Shine, remove the original secret plaintext from the project yourself, or move it to a location the
agent cannot read. Adding a file to `.gitignore` prevents accidental Git inclusion; it does not stop
an agent from reading it.

You can commit `shine.workspace.toml` and shared sources after confirming that sensitive values are
sealed and none remain in `[plain]`. Keep identities, unsealed plaintext, and personal override
files out of commits; ignore `.env.local.shine.toml` and `.env.*.local.shine.toml`. For later changes,
edit the relevant source and seal it again; initialization does not need to be repeated. If another
tool requires dotenv, see [export a workspace to dotenv](./environment.md#export-a-workspace-to-dotenv),
including how to handle plaintext produced by `--include-secrets`.

## What Shine environment protection covers

After sealing, the project stores ciphertext. `shine env run` decrypts and supplies environment
values when it starts the selected program. That program and its children can read the values they
receive, so review the command and the code it executes before running it.

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

The following example belongs in the local `~/.shine/config.toml`:

```toml
secret_backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
age_identity = "~/.shine/age/identity.txt"
```

For shared project settings, use this separate section in `shine.workspace.toml`, replacing the
example recipients with actual member recipients. Keep identity paths in local configuration:

```toml
[env.encryption]
backend = "age"
age_recipients = ["age1tag1qexample...", "age1qteammate..."]
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

`age-plugin-se` generates the identity. Decryption requires the original Mac’s Secure Enclave and
authorization under the selected access policy, using Touch ID by default. Copying its identity
file to another machine is normally insufficient to decrypt.

New Touch ID identities use an `age1tag...` public recipient, so age 1.3 or newer can encrypt on
other platforms without the Secure Enclave plugin. A tagged recipient is more discoverable:
someone who knows it can test whether a ciphertext targets it.

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

Add its `age1...` recipient alongside macOS Touch ID recipients, then reseal existing workspace
payloads on a machine that can decrypt them. Editing the list alone does not update ciphertext.
Follow [Add recipients to existing workspace secrets](./environment.md#add-recipients-to-existing-workspace-secrets)
before sharing the updated files with the new member.

Windows and macOS users can also experiment with [`age-plugin-phone`](https://github.com/biulight/age-plugin-phone)
to authorize decryption with biometrics on their phone. The plugin offers a limited technical Beta with Windows/macOS source installation;
macOS remains experimental. Use only synthetic or disposable data and configure
an independent recovery key before using it. For supported devices, usage limitations, and setup
instructions, see [experiment with phone authorization on Windows and macOS](./environment.md#experiment-with-phone-authorization-on-windows).

When using a normal age identity, protect the identity file and user-directory permissions.
If your team requires hardware keys, use its approved setup and confirm that each member has a
working decryption device and a recovery method.

## Choose a secret backend

Choose according to your team's devices, authorization workflow, and recovery requirements:

| Option | Check before use |
| --- | --- |
| Normal age identity file | Restrict access and keep a safe backup; a program that reads the file may be able to decrypt |
| macOS Touch ID identity | Use a device with Secure Enclave and review system authorization prompts; add a separate recovery recipient |
| GPG identity | Confirm where the private key is stored; for a hardware card, configure and test its PIN or touch requirements |
| Phone authorization | Check the plugin's supported setups; limit use to experimental, disposable data and add an independent recovery key |

A backend name alone is not a complete security guarantee. Hardware keys can restrict private-key
copying, but a program authorized to decrypt still receives plaintext. Follow your team's access
and recovery requirements for production credentials, and test recovery before relying on it.

## Permissions for AI agents

Treat an agent as a capable local collaborator, not as an inherently trusted security boundary:

- Never add `~/.shine/age/identity.txt`, GPG private keys, or cloud credential files to a workspace.
- Do not let an agent keep an interactive shell with high-privilege secrets for long periods.
- Prefer letting the agent edit code and running `shine env run` yourself in a trusted terminal.
- Choose an identity according to the device and access conditions above; limit `age-plugin-phone`
  to its documented experimental scope.
- Cancel unexpected Touch ID, phone biometric, PIN, or YubiKey-touch prompts.

Removing a recipient after an identity leaks, a device becomes untrusted, or a member leaves does not
revoke access to historical ciphertext. Reseal or re-encrypt it and rotate the real upstream token
when necessary:

```bash
shine env secret seal
```

Do not commit environment files containing unsealed strings. Add personal override files to
`.gitignore`, and keep only sealed ciphertext in shared files.

## Hybrid access boundaries

Hybrid encryption lets an authorized GPG or age recipient decrypt the same secrets. It does not
require two-person approval or combine the strength of both identities. Compromise of either
authorized private key can expose the data. Encryption alone does not prove who created a file
or whether it is the latest version.

Removing a recipient and resealing does not revoke historical access, stop authorized readers
copying values, or rotate service credentials. Rotate exposed credentials at their source.
The hybrid encryption format has not undergone an independent security audit.
