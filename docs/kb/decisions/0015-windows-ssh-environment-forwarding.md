# 0015 — Explicit PowerShell environment forwarding for Windows SSH remotes

- **Status**: accepted
- **Evidence**: `cli/src/commands/cli.rs` (`RemoteShell`), `cli/src/ssh/mod.rs`
  (`build_windows_wrapped_remote_command`)

## Context

The default `shine ssh` wrapper is POSIX shell syntax (`env ... sh -c`) and includes a Unix-socket
reverse forward for `shine local`. Windows OpenSSH commonly interprets a remote command through
`cmd.exe`, where that syntax fails and raw secret values would be exposed to incompatible quoting
and metacharacter rules.

## Decision

Provide explicit `--remote-shell windows`; do not auto-detect a remote platform. This mode builds a
PowerShell script using single-quoted literals, encodes UTF-16LE bytes with Base64, and invokes
`powershell.exe -NoProfile [-NoExit] -EncodedCommand <payload>`. The outer remote command contains
no user values. It injects the session hint, terminal theme, and explicit `--with`/`--with-secret`
variables, but creates no listener, `-R` forwarding, token, or transfer context.

## Consequences

- The default POSIX behavior and session transfer protocol remain unchanged.
- Windows remote support is opt-in and has an explicit executable failure if Windows PowerShell is
  unavailable, rather than a misleading POSIX `env` failure.
- `shine local download`, `upload`, and `status` remain unsupported for Windows remotes; extending
  the transfer protocol is a separate design and security task.
