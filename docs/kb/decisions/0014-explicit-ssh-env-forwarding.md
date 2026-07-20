# 0014 — Explicit, secret-separated environment forwarding for `shine ssh`

- **Status**: accepted
- **Evidence**: `cli/src/commands/cli.rs` (`Commands::Ssh`), `cli/src/ssh/mod.rs`
  (`resolve_forwarded_env`, `build_wrapped_remote_command`)

## Context

`shine ssh` already wraps the remote command with `env ... sh -c` to inject its session token,
socket path, and terminal theme without depending on sshd `AcceptEnv`. Users also need selected
values from the active local Shine `[env]` in a remote login shell or one-off command. Forwarding
the entire table would disclose unrelated configuration, while silently applying `env run`'s
secret-first lookup would decrypt and send secrets under an ordinary-looking option.

## Decision

Add repeatable `--with KEY[=ALIAS]` and `--with-secret KEY[=ALIAS]` options before the SSH
destination. `--with` resolves only the exact plaintext key. `--with-secret` alone resolves and
decrypts `KEY_SECRET`, making cross-host secret disclosure an explicit per-invocation choice.
Both forms use the active layered config, reject duplicate/reserved targets, and add safely
single-quoted assignments to the existing remote `env` wrapper. Explicit assignments replace the
environment inherited by the launched remote process; a login shell's own startup files may still
reassign them.

## Consequences

- No sshd configuration or remote Shine installation is required.
- Values are scoped to the remote process tree and are never persisted by Shine on the remote.
- Plaintext and decrypted values appear in the generated SSH command and remote environment, so
  same-user or privileged processes on either host may inspect them. `--with-secret` is the
  deliberate acknowledgement of that exposure; there is no bulk-forward mode.
- Shine-owned `SHINE_SSH_SESSION`, `SHINE_SSH_TOKEN`, `SHINE_SSH_REMOTE_SOCK`, and
  `SHINE_TERMINAL_THEME` cannot be replaced through aliases.
