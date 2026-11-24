# Platform support assessment

Reviewed at `4e2f98a5` on 2026-11-25. The Sys manifests define what ships for each OS or
distribution. The release and CI matrices identify the binary targets and native test hosts.

## Built-in Sys presets

| Platform and manifest | Default selection and installation method | Managed profile shells | Built-in split DNS |
| --- | --- | --- | --- |
| [macOS](../../../presets/sys/macos/shine.toml) | `recommended` includes Rust. Tools use Homebrew, Homebrew Cask, or item-owned installers. | Zsh | Per-domain files under `/etc/resolver` |
| [Ubuntu](../../../presets/sys/ubuntu/shine.toml) | `recommended` includes Rust via rustup and Docker Engine. Items use APT or item-owned installers. `minimal` is the smaller server selection. | Bash, Zsh | systemd-resolved with the local stub enabled |
| [Arch Linux](../../../presets/sys/arch/shine.toml) | Official x86_64 Arch Linux. The only profile is `minimal`, using pacman. The user completes a full system update before installation. | Bash, Zsh | No built-in item |
| [Windows](../../../presets/sys/windows/shine.toml) | `recommended` includes Rust and Neovim. Software items use WinGet. | PowerShell 7 and Windows PowerShell 5.1 | Name Resolution Policy Table (NRPT) |

Use `shine sys list --all` to inspect the active catalog across operating systems and shells.
For the shipped profile contents, see the [built-in selections](../../manual/built-in-presets/index.md#system-presets).
A bootstrap profile selects installation items. `shine sys init` refreshes shell integration
independently of package installation. Where provided, `split-dns` is a managed resource
applied with `shine sys apply split-dns`. [Sys initialization](../decisions/0172-independent-sys-init.md) ·
[Split DNS prerequisites](../../manual/built-in-presets/split-dns.md)

## Other platform boundaries

| Area | What to check |
| --- | --- |
| Self-install and upgrade | On Unix, installing a copy at a protected path can request `sudo`. Self-upgrade needs permission to replace the running executable in its directory. On Windows, use an appropriately privileged terminal for protected locations. [Installation guide](../../manual/installation.md) |
| App and Shell presets | The [generated availability table](../../manual/built-in-presets/index.md#platform-support) lists shipped App categories and Shell commands for macOS, Linux and Windows. The [conformance test](../../../tests/builtin_presets/docs.rs) compares both manual locales with Core discovery. Use the Sys manifests above for distribution-specific support. |
| Shell integration | Unix uses symlinks or managed launchers. Windows uses marked PowerShell and CMD shims. Automatic completion registration supports Bash, Zsh and PowerShell. [Launcher implementation](../../../core/src/runtime/launcher.rs) · [Completion registration](../../../core/src/runtime/profile.rs) |
| Terminal theme | Windows can use an explicit theme or environment hints. The OSC 11 background-color query requires a Unix terminal. [Theme resolution](../../../cli/src/theme/mod.rs) |
| SSH | macOS, Linux and Windows can initiate a session to a compatible POSIX remote with file transfer and on-demand secret release. With `--remote-shell windows`, Shine forwards selected environment values through PowerShell. Transfer and on-demand release require a POSIX remote. [SSH transport and remote modes](../../manual/guides/ssh-transfer.md) |
| Hardware-backed identities | Touch ID uses the macOS Secure Enclave with `age-plugin-se`. Phone authorization on macOS and Windows uses the experimental external age plugin. Shine has no built-in Windows Hello backend. [Identity setup](../../manual/guides/environment.md#use-age-identities) · [Windows Hello decision](../decisions/0032-defer-windows-hello-to-external-age-plugin.md) |
| Legacy HTTP service | Shine 2.4.0 retired the server and its installation commands. `shine clear serve` stops and removes the old launchd, systemd user or Windows Task Scheduler registration and preserves published files. [Cleanup implementation](../../../cli/src/legacy_serve.rs) · [Migration guide](../../manual/guides/upgrading.md#retire-an-older-local-http-service) |

## Verification coverage

| Evidence | What it establishes |
| --- | --- |
| [Workspace test workflow](../../../.github/workflows/test.yml) | The native Rust test targets are Ubuntu x86_64, macOS arm64 and Windows x86_64. Offline installer tests also run on all three OS families. |
| [Release packaging](../../../.github/workflows/package-assets.yml) | The matrix builds x86_64 and aarch64 archives for macOS, GNU/Linux and Windows. Linux archives are checked against Ubuntu 22.04 / glibc 2.35. The x86_64 binary runs natively and aarch64 runs under QEMU for version, help and schema checks. |
| [Stable release verification](../../../.github/workflows/release.yml) | The `verify-binstall` job checks package resolution for all six targets and installs and runs the binary on the three native test architectures. |
| [Arch preset regressions](../../../tests/builtin_presets/arch.rs) | Fixtures exercise the pacman invocation, detection, reruns and shell integration. Live package installation, mirrors and signatures require separate acceptance on official x86_64 Arch. [Arch scope](../decisions/0167-arch-minimal-bootstrap.md) |

For a compatibility claim about an external integration, record the host, operation and observed
result. The 2026-10-11 macOS check observed removal of the legacy launchd
registration. Native Linux and Windows service-manager
cleanup still need their own smoke results. Apply the same distinction to privileged DNS
changes, package installations, terminal probes, SSH peers and hardware authorization.

When support changes, update the owning manifest or platform implementation and its regression
coverage. Refresh the generated App/Shell table in both manual locales when availability changes.
Update this assessment when a platform boundary or the evidence for it changes.
