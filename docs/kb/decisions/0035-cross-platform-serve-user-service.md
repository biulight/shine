# 0035 — Use native per-user persistence for the local HTTP server

- **Status**: Accepted
- **Date**: 2026-08-27
- **Evidence**: `cli/src/serve.rs`, `docs/kb/architecture/platform-support.md`

## Context

`shine serve start` is portable, but persistent registration originally existed only as a macOS
launchd agent. Linux and Windows need equivalent lifecycle commands without administrator access,
a second HTTP implementation, or per-app daemons. The registration also has to preserve a custom
Shine state directory after the installing process and its environment are gone.

## Decision

- macOS continues to use one launchd user agent.
- Linux installs one unit under the standard user configuration directory
  (`$XDG_CONFIG_HOME/systemd/user/`, falling back to `~/.config/systemd/user/`) and manages it with
  `systemctl --user`.
- Windows installs one Task Scheduler entry for the current interactive user. It triggers at logon,
  runs at the limited privilege level, and is started immediately after registration.
- Every platform invokes the same `shine serve start` foreground implementation and passes the
  resolved Shine directory through `--config-dir`.
- Status and uninstall query only the native per-user registration. No platform requires root,
  `sudo`, an elevated terminal, or a system account.

## Consequences

- `serve install/status/uninstall` has intentional macOS, Linux, and Windows implementations.
- Linux output is retained by the per-user journal. macOS keeps stdout/stderr below
  `<shine_dir>/run/http/`; the Windows task does not create a separate log file.
- Windows persistence applies while the installing user has an interactive login, matching the
  loopback server's user-scoped purpose and avoiding stored credentials.
- Generated launchd, systemd, and Windows command lines require platform-specific escaping tests;
  those tests must not mutate the developer's real service registry.
