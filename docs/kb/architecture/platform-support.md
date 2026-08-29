# Platform Support Assessment

Last assessed: 2026-08-27

This document records the current implementation coverage and the remaining gaps across macOS,
Ubuntu, and Windows. It is an implementation reference, not a public compatibility promise. Verify
the relevant code, presets, CI workflows, and public manual again before changing behavior or
announcing support.

## Executive summary

Shine has a genuine cross-platform core rather than a Unix implementation that merely compiles on
Windows. Configuration, environment handling, manifest-tracked install/uninstall, shell preset
lifecycle, self-update, task execution, and most status/update flows have explicit Windows paths.
Windows shims, PowerShell profiles, path comparison, CRLF/BOM handling, WinGet proxying, and NRPT
split DNS are implemented deliberately.

The overall maturity remains uneven:

| Platform | Current maturity | Main strengths | Main gaps |
| --- | --- | --- | --- |
| macOS | Highest | Broadest built-in sys preset, Homebrew/Cask integration, zsh profile, launchd service install, Secure Enclave/Touch ID, native split DNS | Managed system profile targets zsh; some tty behavior requires real-terminal verification |
| Ubuntu | High | Rich server/developer bootstrap, bash and zsh profiles, minimal server profile, systemd user service, systemd-resolved safety checks | More installer logic lives in per-item scripts; no built-in Rust bootstrap |
| Windows | Medium-high core, lower advanced coverage | PowerShell shims and profiles, WinGet, Windows path and line-ending handling, NRPT split DNS, Windows OpenSSH environment wrapper, persistent HTTP task | No Windows-remote transfer or secret broker, no terminal OSC theme detection, thinner editor bootstrap, and no native Windows test job in normal CI |

The highest-priority remaining engineering gap is:

1. Normal CI runs the Rust test suite only on Ubuntu. macOS and Windows release jobs prove that the
   binaries compile, but do not execute platform-specific tests.

## Capability matrix

`Full` means the feature has an intentional implementation for the platform. `Partial` means the
core path works but the platform lacks a related integration or advanced mode. This table describes
the repository at the assessment date, not every possible external preset.

| Capability | macOS | Ubuntu | Windows | Notes |
| --- | --- | --- | --- | --- |
| Official binary and install script | Full | Full | Full | Release assets cover x86_64 and aarch64 for Darwin, Linux, and Windows |
| Self install and upgrade | Full | Full | Full | Unix can use `sudo`; Windows requires an already elevated terminal for protected destinations |
| Config, env, workspace, and task lifecycle | Full | Full | Full | Secret backend availability still depends on installed external tools |
| Shell preset lifecycle | Full | Full | Full | Unix uses symlinks or managed launchers; Windows uses marked `.ps1` and `.cmd` shims |
| Built-in shell commands | Full | Full | Partial | All shared commands are available except Unix-only `copyfile`; platform-specific proxy scripts exist |
| App lifecycle engine | Full | Full | Full | Copy, transforms, JSON merge, generators, hooks, artifacts, backup, upgrade, and uninstall are shared |
| Built-in App/Shell availability filtering | Full | Full | Full | Exact `macos`/`linux`/`windows` selectors are supported; `unix` remains a compatibility group, and a generated bilingual manual block is conformance-tested against runtime metadata |
| Dynamic completions | Bash/Zsh | Bash/Zsh | PowerShell | Fish and Elvish profiles exist in generic shell code but dynamic completion registration is not provided |
| Sys bootstrap | 21 items | 17 items | 14 items | Counts include the independent managed `split-dns` item |
| Package provider | Homebrew/Cask | APT, Homebrew, scripts | WinGet | Ubuntu uses the largest number of item-owned compatibility scripts |
| Managed split DNS | Full | Full | Full | Uses `/etc/resolver`, systemd-resolved drop-in, and NRPT respectively |
| Terminal theme auto-detection | Full | Full | Partial | OSC query is Unix-only; Windows can consume an existing or explicitly supplied theme value |
| SSH to a POSIX remote | Full | Full | Full as local initiator | Requires a compatible Shine on the POSIX remote |
| SSH to a Windows remote | Partial | Partial | Partial | Safely injects selected environment values through PowerShell |
| SSH transfer and on-demand secret broker | Full for POSIX remote | Full for POSIX remote | No Windows-remote mode | Windows remote mode has no compatible transfer/control channel |
| `serve start` foreground server | Full | Full | Full | Shared Tokio loopback server |
| `serve install/status/uninstall` | Full | Full | Full | Uses launchd, a systemd user unit, and a current-user scheduled task respectively |
| Secure Enclave/Touch ID age identity | Full | Not applicable | Missing by design | Windows Hello integration was deferred to an external age plugin in ADR 0032 |

## System preset differences

### Common coverage

All three built-in sys presets provide Rust, Neovim, Yazi, Starship, zoxide, Atuin, fzf, bat, eza,
ZeroTier, split DNS, Bun, pnpm, and mise. Package installation is presence-oriented: Shine
bootstraps missing software but does not own third-party version upgrades.

### macOS

The macOS preset contains 21 items. In addition to the common set it includes Homebrew, AstroNvim,
nvm, Fastfetch, and several zsh plugins. Most installation is declarative through Homebrew or
Homebrew Cask, with scripts reserved for Homebrew, Rust, and AstroNvim compatibility flows. The
generated managed system profile targets zsh.

### Ubuntu

The Ubuntu preset contains 17 items and provides `recommended`, `all`, and `minimal` profiles. The
minimal profile is useful for servers and deliberately omits prompt, history-sync, JavaScript, and
Homebrew tooling. The default profile includes Rust through the official rustup installer. Ubuntu
supports both bash and zsh managed profiles.

Ubuntu has the richest script-based bootstrap surface. AstroNvim, Atuin, Yazi, Starship, zoxide,
zsh-vi-mode, bat, eza, Bun, pnpm, mise, Homebrew, and ZeroTier use item-owned scripts, while Neovim
and fzf use APT. Rust uses the same official rustup bootstrap model as macOS. This gives better
control over upstream versions and compatibility, but also creates the highest ongoing maintenance
and external-download test burden.

### Windows

The Windows preset contains 14 items and provides `required`, `recommended`, and `all` profiles.
It uses declarative WinGet packages and PowerShell profile fragments. Rust and Neovim are included,
while AstroNvim is not. WinGet proxy support passes an explicit `--proxy` argument because WinGet
does not honor the standard proxy environment variables by itself.

Windows profile integration intentionally updates both PowerShell 7 and Windows PowerShell 5.1
profile files and preserves an existing leading BOM. Shell launchers use managed `.ps1` and `.cmd`
files instead of depending on Windows symlink privileges.

## Confirmed gaps and risks

### Closed 2026-08-27: App presets can express an exact operating system

ADR 0034 added exact `macos`, `linux`, and `windows` selectors while retaining `unix` as the
macOS/Linux compatibility group. App destination maps prefer an exact branch over `unix`, App and
Shell file filters share the same vocabulary, and host-independent validation checks all three
effective OS branches plus every explicitly declared destination. The built-in Surge category now
has only a macOS destination and is absent from runtime App candidates on Linux and Windows.

### P1: CI does not execute native macOS or Windows tests

The reusable test workflow runs only on `ubuntu-latest`. Preview and release asset packaging builds
on macOS and Windows runners, which catches compile and linker failures, but does not compile or run
the platform-gated test modules as test targets. Normal pull-request CI does not invoke the package
matrix either.

This leaves the highest-risk platform-specific paths under-tested at merge time: Windows shims,
PowerShell profile rewriting, BOM and CRLF preservation, Windows path normalization, atomic
replacement semantics, NRPT convergence, Windows OpenSSH wrappers, and macOS tty behavior.

Acceptance criteria:

- Pull requests run native Rust tests on Ubuntu, macOS, and Windows x86_64.
- Platform-specific tests compile and execute on their owning OS.
- Release cross-builds remain responsible for the additional aarch64 asset targets.
- At least one Windows smoke test exercises managed shim creation/removal and both PowerShell
  profile paths.
- Real-terminal-only macOS behavior remains documented when it cannot be made deterministic in CI.

### Persistent HTTP service integration

`serve install/status/uninstall` uses launchd on macOS, `systemctl --user` on Linux, and a
least-privilege, current-user Task Scheduler entry on Windows. Every registration starts the shared
foreground server with an explicit `--config-dir`, so custom Shine state remains attached after the
installing shell exits. See ADR 0035.

### P2: Windows remote SSH is intentionally feature-reduced

Windows remote mode safely selects PowerShell, injects environment values, and loads the normal
interactive profile. It does not provide `shine local` upload/download/status or the on-demand
secret broker because those depend on the POSIX transfer/control channel.

Before expanding this area, decide whether Windows remote support is intended to remain a focused
environment-forwarding mode or reach parity with POSIX remotes. A parity implementation needs a
separate security and transport design; it should not reuse POSIX shell syntax or assume Unix-domain
sockets.

### P2: Terminal theme synchronization is not symmetric

The OSC 11 terminal query implementation is Unix-only. Managed macOS and Ubuntu profiles can detect
and publish the terminal theme automatically. Windows may use an already exported
`SHINE_TERMINAL_THEME`, `COLORFGBG`, or an explicitly configured value, but does not query the
terminal through the same path.

Treat this as a usability gap, not a core correctness gap. Any Windows implementation needs
real-terminal verification comparable to the macOS `/dev/tty` investigation recorded in
`lessons.md`.

### P2: Built-in bootstrap profiles are not feature-equivalent

Raw item counts are not themselves a compatibility goal because each OS has different package and
shell needs. All three default `recommended` profiles now share a Rust toolchain, Neovim editor, and
the common terminal-tool baseline. The remaining useful parity gaps are:

- Windows has no AstroNvim bootstrap.
- Windows lacks a server-oriented minimal profile.
- Ubuntu's large script surface needs more integration coverage than declarative providers.

Add items only when their detection, installation ownership, profile integration, proxy behavior,
and non-upgrade semantics can be tested. Do not force identical profile membership merely to make
the counts match.

## Implementation sequence

1. Add native macOS and Windows test jobs to pull-request CI before expanding more platform-specific
   behavior.
2. Add Ubuntu and Windows persistent service integrations if stable local HTTP resources are a
   supported workflow on those platforms.
3. Decide the target scope for Windows remote SSH, then either document the intentional boundary or
   design a secure Windows transfer/broker transport.
4. Fill remaining high-value sys preset gaps, starting with a decision on Windows AstroNvim and a
   server-oriented Windows profile, with platform smoke tests.
5. Reassess Windows terminal theme detection after the higher-impact correctness and CI gaps are
   closed.

## Cross-platform definition of done

For a change that claims support on one or more of these platforms:

1. Encode availability in metadata or an explicit platform boundary; do not rely on a destination
   path or external command failing on the wrong OS.
2. Unit-test platform-independent parsing for every supported branch.
3. Run platform-gated tests on the owning OS in CI.
4. Verify install, status/update, upgrade, and uninstall behavior, including modified-user-file and
   dry-run cases where applicable.
5. Verify native path syntax, line endings, profile location, executable suffixes, and privilege
   behavior.
6. Add a real-OS smoke test for external package managers, services, DNS, terminals, or SSH where a
   unit test cannot reproduce the OS contract.
7. Update both public manual locales in the same change for any user-visible compatibility or
   command/schema change.
8. Regenerate the built-in preset platform capability block when App destinations or App/Shell
   file selectors change; the Rust conformance test checks both manual locales.
9. Update this assessment when a listed gap is closed or its intended scope changes.

## Evidence map

| Area | Authoritative implementation or documentation |
| --- | --- |
| Platform and release-target mapping | [`cli/src/platform.rs`](../../../cli/src/platform.rs) |
| App platform filtering | [`core/src/runtime/app_metadata.rs`](../../../core/src/runtime/app_metadata.rs) |
| Shell launchers and Windows shims | [`cli/src/bin_links.rs`](../../../cli/src/bin_links.rs) |
| Shell and PowerShell profile locations | [`cli/src/shells/mod.rs`](../../../cli/src/shells/mod.rs), [`cli/src/shells/profile.rs`](../../../cli/src/shells/profile.rs) |
| System preset manifests | [`presets/sys/macos/shine.toml`](../../../presets/sys/macos/shine.toml), [`presets/sys/ubuntu/shine.toml`](../../../presets/sys/ubuntu/shine.toml), [`presets/sys/windows/shine.toml`](../../../presets/sys/windows/shine.toml) |
| Package provider routing | [`cli/src/sys/bootstrap.rs`](../../../cli/src/sys/bootstrap.rs) |
| Cross-platform split DNS | [`core/src/runtime/sys.rs`](../../../core/src/runtime/sys.rs) |
| Terminal theme resolution | [`cli/src/theme/mod.rs`](../../../cli/src/theme/mod.rs) |
| SSH platform boundary | [`cli/src/ssh/mod.rs`](../../../cli/src/ssh/mod.rs), [`docs/manual/guides/ssh-transfer.md`](../../manual/guides/ssh-transfer.md) |
| HTTP service platform boundary | [`cli/src/serve.rs`](../../../cli/src/serve.rs) |
| Test workflow | [`.github/workflows/test.yml`](../../../.github/workflows/test.yml) |
| Release asset matrix | [`.github/workflows/package-assets.yml`](../../../.github/workflows/package-assets.yml) |
| Windows Hello decision | [`decisions/0032-defer-windows-hello-to-external-age-plugin.md`](../decisions/0032-defer-windows-hello-to-external-age-plugin.md) |

## Assessment verification record

At the 2026-08-27 assessment point, `cargo test --target-dir target` passed 937 tests on an
aarch64 macOS host. Ubuntu and Windows behavior in this document was assessed from implementation,
presets, tests, workflows, and existing platform lessons; no end-to-end bootstrap was run on an
Ubuntu or Windows machine during this assessment.
