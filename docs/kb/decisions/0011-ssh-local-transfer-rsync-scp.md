# 0011 — `shine local` transfers via local-initiated rsync/scp

- **Status**: accepted
- **Evidence**: `cli/src/ssh/protocol.rs` (`Transfer`/`Starting`/`Log`/`Done`,
  `PROTOCOL_VERSION = 2`), `cli/src/ssh/agent.rs` (`handle_transfer`,
  `build_transfer_argv`, `choose_tool`, `resolve_tool_binary`),
  `cli/src/ssh/remote_client.rs` (`absolutize_remote_spec`, `relay_until_done`),
  `cli/src/ssh/session_context.rs` (`SessionContext`), `cli/src/ssh/mod.rs`
  (ControlMaster injection in `build_ssh_invocation_args`),
  `cli/src/commands/local.rs` (`--scp`)

## Context

The original `shine local download/upload` (ADR-less; see
`docs/ssh-local-transfer-prd.md`) hand-built a byte-streaming protocol over the
`shine ssh` reverse tunnel: the local agent read/wrote files itself and staged
directories as tar archives. That reimplemented, badly, what `rsync` already
does — no delta sync, no resume, no wildcard/glob support (the original
complaint: `shine local download '*.log'` failed), and a growing burden of
directory-semantics, overwrite-policy, and progress code. Wrapping scp/rsync in
the *remote-initiated* direction is impossible (the remote would have to log
back into the local machine), but the local machine **can** reach the remote —
it dialed out to it — so a **local-initiated** wrapper is viable. The user
always connects via `~/.ssh/config` host aliases, which makes reconstructing an
equivalent connection trivial.

## Decision

`shine local` stops moving bytes over the tunnel. The tunnel now carries a
**control + log-relay** channel (`PROTOCOL_VERSION` bumped to 2). The remote
sends one `Transfer { direction, remote_spec, local_spec, force, dry_run,
use_scp }`; the local agent validates the session token, resolves the local
side against the session directory, and spawns `rsync` (default) or `scp` on the
local machine, which opens its **own** ssh connection to the host and moves the
data directly. The child's stdout/stderr are relayed back verbatim as `Log`
frames, followed by `Done { code }`, and the remote propagates that exit code as
its own.

- **Default rsync, `--scp` to force scp.** When rsync is unavailable (locally,
  or — probed cheaply over the control master — on the remote) shine
  **auto-falls back to scp and prints a notice**.
- **ControlMaster reuse.** At `shine ssh` time shine injects
  `-o ControlMaster=auto -o ControlPath=<session_dir>/ctl.sock -o
  ControlPersist=60` (unless the user set their own multiplexing). The rsync/scp
  child reconnects over that master, so there is **no second authentication /
  2FA prompt**, and the ssh reconnection options are our own UUID ControlPath —
  never anything from the wire.
- **Wildcards.** Remote-owned globs are expanded by rsync/scp's remote shell
  (the remote spec is anchored to the remote cwd with metacharacters preserved,
  never canonicalized); local-owned (upload-source) globs are expanded with the
  `glob` crate.
- **Session context** (`host`, `ssh_options`, `local_dir`, `control_path`) is
  captured from `split_ssh_args`/cwd and held as an in-memory `Arc`, also
  persisted to `<session_dir>/context.toml` for diagnostics.

## Consequences

- Native rsync delta/resume/compression, directory/symlink/permission handling,
  and glob support — none of it reimplemented in shine.
- **New attack surface**: the agent now executes external tools with argv partly
  derived from wire-supplied (untrusted, token-leakable) strings. Mitigations
  (all in `agent.rs`): argv only, never a shell; the remote operand is always
  the single token `<host>:<remote_spec>` after a `--` separator (so a hostile
  `-oProxyCommand=…` becomes an inert `host:-oProxyCommand=…`); local operands
  anchored to the session dir and `./`-prefixed if dash-leading; the `-e`/`-o`
  reconnection string built solely from `SessionContext`; tilde-only expansion
  for wire paths; local glob via the `glob` crate (no shell). See
  `docs/kb/lessons.md`.
- **Overwrite semantics change**: rsync always overwrites, so no-`--force` maps
  to rsync `--ignore-existing` (skip, never clobber); scp cannot gate overwrite
  at all, so the scp path without `--force` prints a warning.
- **Second-connection auth**: reauth-free for the alias/key + ControlMaster
  case; a password/2FA host without a control master would prompt on the *local*
  terminal. Windows OpenSSH ControlMaster support is limited — key auth
  recommended there.
- **Progress fidelity**: progress is rsync/scp's own (`--info=progress2`),
  relayed as raw chunks (preserving `\r` redraws) — close but not identical to
  the old custom bar.
- **Removals**: `cli/src/ssh/dir_transfer.rs` (tar staging) and the byte-stream
  variants + `copy_exact` in `protocol.rs` are gone; `PROTOCOL_VERSION` → 2.
