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
PowerShell script using single-quoted literals and encodes UTF-16LE bytes with Base64. A separately
encoded Windows PowerShell bootstrap uses `Get-Command` to prefer `pwsh.exe` (PowerShell 7), falling
back to `powershell.exe` (Windows PowerShell 5.1) when `pwsh.exe` is unavailable. Interactive
sessions invoke the selected shell with `-NoExit -EncodedCommand <payload>` so its normal profile
loads; explicit remote commands use `-NoProfile -EncodedCommand <payload>` for deterministic
execution. The outer remote command stays a single `powershell.exe -EncodedCommand` invocation
with no CMD operators, so it also works when
the SSH server's default shell is already PowerShell rather than CMD. Shell selection happens
before the payload runs, and the bootstrap invokes it exactly once and propagates its exit code.
The outer remote command contains no user values. It injects the session hint, terminal theme, and
explicit `--with`/`--with-secret` variables, but creates no listener, `-R` forwarding, token, or
transfer context.

## Consequences

- The default POSIX behavior and session transfer protocol remain unchanged.
- Windows remote support is opt-in, prefers the cross-platform PowerShell 7 runtime when installed,
  and retains compatibility with stock Windows PowerShell 5.1.
- Interactive sessions load the user's normal PowerShell profile, making Shine's managed PATH and
  source-command wrappers such as `setproxy` available; explicit remote commands remain isolated
  from profile side effects.
- `shine local download`, `upload`, and `status` remain unsupported for Windows remotes; extending
  the transfer protocol is a separate design and security task.
