# Changelog

All notable changes to this project will be documented in this file.
See [Conventional Commits](https://www.conventionalcommits.org/) for commit guidelines.

---

## [Unreleased]

## [1.1.2] — 2026-08-06

### Changed

- Corrected the minimum supported Rust version to 1.88 and made release asset builds and crates.io
  publishing wait for the reusable MSRV check.

## [1.1.1] — 2026-08-06

### Bug Fixes

- Declared Rust 1.86 as the minimum version. This was corrected to Rust 1.88 in the next release
  after the resolved `jsonc-parser` dependency proved to require it.
- Pinned `jsonc-parser` to `0.32.3`.

## [1.1.0] — 2026-08-05

### Features

- Added external shell preset deployment modes and a compact `shine list` column layout.

### Bug Fixes

- Updated the Qwen model mapping used by the built-in agent preset.

### Internal

- Published the CLI as `shine-cli` and the reusable, UI-agnostic shared library as `shine-core`.

## [1.0.0] — 2026-08-02

### Breaking Changes

- Simplified the everyday CLI around canonical `app/`, `shell/`, and `sys/` targets: top-level
  install/uninstall now share one target grammar, `list --available` provides a unified catalog,
  `info` can inspect uninstalled presets, and `upgrade [TARGET]` applies one selected resource.
- Replaced the `reinstall` flow with `install --replace-managed`, and grouped advanced operations
  under `preset new`, `app artifact`, `env secret`, and `sys bootstrap`. Superseded pre-release
  spellings are not retained as aliases.
- Standardized read commands around `list`, `info`, `status`, and `get`: `env show` is now
  `env list`, identity inspection is now `env secret identity list`, and overlay inspection is
  now `preset overlay info`.
- Grouped preset source management under `shine preset`: use `preset export`, `preset link`,
  `preset unlink`, `preset overlay ...`, and `preset pull` instead of the former top-level
  commands.
- Replaced the ambiguous `shine clear` command with `shine state migrate`, and renamed
  `update --refresh` to `update --refresh-release`.
- Removed all compatibility aliases for the superseded command names.

### Features

- Standardized `shine --version` on Cargo-style provenance output
  (`shine 1.0.0 (<commit> <date>)`) and the matching `1.0.0-preview` label for preview builds.
- Added `shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>` with preset metadata, runtime
  requirements, declared environment names, and installation status.
- Added explicit `shine info sys/<ITEM>` routing for system item details.
- Added `shine preset copy <kind>/<name>` to copy one complete built-in preset into the current
  directory as an overlay customization starting point.
- Added optional fixed working directories for personal tasks through `shine task save --cwd`,
  while existing tasks continue to run from the caller's current directory.
- Expanded dynamic shell completion to installed and canonical targets, app artifact/refresh
  categories, recorded system-update items, and saved task names; completion path discovery is
  shared with the config module and remains read-only.

### Docs

- Reworked CLI help and documentation around the 1.0 command vocabulary and hierarchy.

## [0.40.0] — 2026-08-01

### Features

- Added generic app-file generators whose output participates in normal install, update, and
  upgrade hashing and safety checks, plus `shine app refresh <app-id> [file]` for explicitly
  refreshing manual generators without consuming short-lived provider access during routine
  update checks.
- The Surge preset can now fetch a Base64 URI subscription, convert compatible SS/VMess nodes
  into a `policy-path`-backed `Subscription` group, and retain the last-known-good generated file
  when a refresh fails.
- Moved the Surge profile include patch/unpatch artifact into the built-in Bun preset, with atomic
  writes, permission and line-ending preservation, symlink protection, and symmetric teardown.
- Reworked `ccenv` as a cross-platform Bun launcher for Claude Code. It now defaults to Codex
  through CLIProxyAPI, also supports DeepSeek and Qwen, forwards Claude arguments unchanged, and
  scopes provider credentials and model settings to the launched process.
- Added managed system configurations to `shine list` and expanded verbose update/upgrade output
  so skipped, current, and attention-needed resources are visible alongside available updates.
- Removed support for legacy project-local `config.toml` and `.env.toml` filenames; projects must
  use `shine.config.toml` and `shine.env.toml`.
- Removed automatic migration of the former global `~/.shine/env.toml`. Before upgrading, run a
  v0.39 binary once to migrate it, or move/merge its values into `~/.shine/shine.env.toml`.

### Bug Fixes

- Fixed update detection for changed or renamed embedded shell sources, including stale pre-Bun
  `ccenv` installations.
- Hardened app generators with explicit trust gates, bounded execution, HTTPS-only subscription
  URLs, environment allowlists, and credential-free diagnostics.

### Internal

- Added Bun tests and strict type-checking coverage for the cross-platform `ccenv` launcher and
  the Surge subscription/profile tooling.

### Docs

- Documented generated app files, explicit generator refresh, the built-in Surge artifact,
  process-scoped Bun `ccenv`, managed system status visibility, and the v0.40 config migration.

## [0.39.0] — 2026-07-19

### Features

- Added Git-managed preset overlays with depth-one force mirroring, so a personal overlay can be linked, pulled, inspected, and cleanly unlinked without replacing the main preset source.
- Reworked `shine ssh` / `shine local` transfers to use local-initiated `rsync` by default with `scp` fallback, SSH connection reuse, streamed native progress, glob support, overwrite protection, and dry-run previews. The local agent now supports Windows through a loopback TCP forward while Linux and macOS continue to use Unix sockets.
- Added explicit `--with KEY[=ALIAS]` and `--with-secret KEY[=ALIAS]` forwarding to `shine ssh`, including Windows session support, so selected local config values can be injected into a remote session without forwarding the whole environment.
- Added `post_install` hooks and explicit artifact teardown through `shine app unbuild`; app uninstall now runs teardown on a best-effort basis. The built-in Surge preset can manage proxy-group includes, and the new Clash Verge preset renders the active CVR 2.x subscription's Merge, proxy, proxy-group, and rule editor files.
- Added Bun-powered shell preset entries, including metadata/header descriptions and optional Shine env injection through generated managed launchers.
- Added `shine theme sync` to detect the active terminal background and export matching `SHINE_TERMINAL_THEME` / `BAT_THEME` values, with managed shell helpers for Unix and Windows.
- Added a minimal Ubuntu system-init profile for production servers, proxy-aware `shine sys init`, and read-only software update checks for Homebrew, apt, and winget, including proxy status reporting.
- `shine env show` now groups values by their configuration source.

### Bug Fixes

- Fixed Windows split-DNS reconciliation so normalized NRPT query values remain idempotent.
- Fixed Windows SSH sessions to load their PowerShell profiles before applying forwarded environment values.
- Fixed `shine sys init --proxy` so winget receives the configured proxy while update checks remain read-only.
- Fixed terminal-theme OSC 11 reads on macOS and Ghostty by using deadline-based `select(2)` reads with the correct terminal mode.
- Fixed environment writes so `env set`, `env encrypt`, and `env delete` refuse to silently write beneath a higher-priority override.
- Fixed Clash Verge subscription rendering for enhanced profiles and platform-specific installation/theme code for Windows builds.

### Internal

- Pinned Rust and Bun tooling with mise, added strict TypeScript checks for Bun presets, and refreshed dependencies.

### Docs

- Documented Git-managed overlays, the rsync/scp SSH transfer design, explicit SSH environment forwarding, Bun shell presets, Clash Verge subscription rendering, and terminal theme synchronization.

## [0.38.0] — 2026-07-12

### Features

- Added `shine app build <app-id>`, which runs an app preset's `[artifact].script` with a fixed `SHINE_APP_*` env contract plus the active `[env]` table passed as stored (no decryption). It never runs implicitly during `install`/`upgrade`, and script failures propagate as errors. The built-in `surge` preset was repurposed around this: `app install surge` copies `local-proxies.conf`/`local-rules.conf` into the Surge Profiles dir, and `app build surge` (via an overlay `build.sh`) patches the active profile's `[Proxy]`/`[Rule]` `#!include` lines.
- App categories can now declare a `post_upgrade` hook that runs after `shine upgrade` actually installs or updates at least one file in that category, plus a local HTTP server (`shine serve install/start/status/uninstall/url`) for serving app-managed resources under `~/.shine/http/`, including revalidation of previously served content.
- Added `shine task save/run/list/info/delete` and a top-level `shine run` alias for saving and directly executing personal shortcut commands (argv-based, no shell, exit code passed through verbatim).
- `shine self install`/`self upgrade` now auto-elevate via sudo when the destination isn't user-writable, instead of asking the user to manually re-run with sudo.

### Bug Fixes

- Fixed `shine sys` profile reconciliation (three-way merge and sentinel-block matching) to be CRLF/LF-agnostic, so a Windows-edited profile file no longer gets silently rewritten to LF on every run.
- Fixed `shine upgrade` section spacing (blank lines only between sections that actually printed something) and normalized checkmark-line indentation across Shell Presets/App Configs/Managed System Configs output.
- Fixed post_upgrade hook stdout to be opt-in (`show_output`) instead of an implicit "last hook in the sequence" heuristic, so unrelated hook success output no longer surfaces as a note.
- Fixed `shine info`'s app-file Source line to resolve through an active overlay, matching the shell-file equivalent.
- Fixed `shine sys`/self-install/completion to resolve the target home directory through the sudo-aware resolver instead of raw `HOME`, fixing the wrong home directory being used under sudo.
- Fixed color detection for warnings printed to stderr (e.g. during `shine update`) to check stderr support instead of stdout's.
- Fixed misaligned presets-note labels and managed system config item indentation in various status output.

### Internal

- Extracted a shared `install_core` module and a `preset_meta` core shared by app and shell metadata parsing; added a generic TOML manifest persistence module and a shared sentinel-block module used by both shell and sys profile handling.
- Split several large modules for cohesion: `update_check.rs`, `sys/commands.rs`, `sys/profile.rs`, `apps/mod.rs`, `shells/mod.rs`, and `show.rs`.
- Consolidated duplicated shell-quoting logic and test-helper setup, and documented that the ~800-line file-size guideline counts production code only (inline `#[cfg(test)]` modules are exempt).

## [0.37.0] — 2026-07-09

### Features

- Added `shine ssh` and `shine local download`/`upload`/`status` for transferring files and directories between machines over a session-scoped channel piggybacked on an interactive SSH session, with throttled progress output, directory transfers (tar-streamed, symlink-escape and traversal checks), overwrite protection, and `--dry-run` previews. The local side also runs on Windows (the remote host is always assumed Linux/macOS).
- Added an `age` secret backend for `shine env encrypt`/`decrypt`/`seal`, supporting multi-recipient encryption and Apple Touch ID (Secure Enclave) identities via `age-plugin-se`. Ciphertext is tagged so existing GPG secrets keep decrypting unmodified.
- Added `shine env identity init`/`show` to generate and inspect age identities, including `--touch-id` for Secure Enclave identities on macOS.
- `-r/--recipient` is now repeatable and `--backend gpg|age` selects the secret backend for `shine env encrypt` and `shine env seal`; `age_recipients` and `age_identity` config keys were added alongside the existing `gpg_key_id`/`secret_backend`.
- `shine update` now reports pending Git-managed preset and overlay pulls alongside pending config changes.

### Bug Fixes

- Fixed `shine sys init`/`upgrade` prompting for sudo even when nothing needed to change, by checking each admin-required item's up-to-date state before requesting authorization.
- Fixed `shine sys` split-DNS setup on Ubuntu to detect a disabled `systemd-resolved` stub listener and explain why split DNS can't take effect instead of reporting a false converge.
- Fixed `shine sys` drift detection for managed system resource environment variables.
- Fixed `shine info` shell diffs to use the same source `shine update` uses.

### Internal

- Split `secret.rs` into a `SecretBackend` trait plus GPG and age implementations, routed by ciphertext tag.
- Further split the `config`, `sys`, and top-level command-handling modules for cohesion (env commands and preset shims out of `main.rs`; sys data model and run-manifest out of `sys/mod.rs`).

### Docs

- Documented `shine ssh` / `shine local` and the age secret backend in the English and Chinese READMEs.
- Added ADR 0008 for the tagged-ciphertext secret backend routing decision.

## [0.36.0] — 2026-07-05

### Features

- Added `shine env seal` and `shine env run` for editable, layered workspace environment files with GPG-sealed secrets and transparent mode-specific caching.
- Added `shine env run --with` for injecting selected encrypted or plaintext config values into a child process without modifying the current shell.
- Added environment variable descriptions, sensitive-value redaction, preset-provided metadata, and detailed inline `{ value, description }` overrides.
- Added managed system resource drivers and expanded `shine sys list` / `shine sys info` to expose available init and managed items with their status and setup commands.
- Added overlay environment overrides and made overlays compose with embedded or external preset sources, including overlay-only categories.
- Added `shine pull` plus `update --pull` and `upgrade --pull` for safe, fast-forward-only updates of Git-managed preset and overlay sources.

### Bug Fixes

- Fixed project configuration discovery so sparse project settings inherit global values while retaining the documented source-priority behavior.
- Fixed privileged app uninstall state and serialized administrator-level filesystem operations across concurrent processes.
- Fixed filtered app category resolution when categories or metadata are supplied by an overlay.
- Fixed proxy status checks so environment changes supplied by an overlay are detected.

### Deprecations

- Legacy project `config.toml` and `.env.toml` filenames now warn that support will be removed in v0.40.0; rename them to `shine.config.toml` and `shine.env.toml`.

### Internal

- Split the CLI library, command handlers, and app, shell, system, and configuration implementations into focused modules without changing their command routing.

### Docs

- Expanded the English and Chinese READMEs for workspace environments, managed system resources, composable overlays, detailed environment values, and Git-managed preset updates.
- Added the maintainer knowledge base covering architecture, invariants, decisions, conventions, operations, and lessons learned.

## [0.35.0] — 2026-07-04

### Features

- Added the cross-shell `shine-env-export` helper for loading encrypted or plaintext Shine env values into the current shell session.
- Added `shine env export <KEY> --as <ALIAS>` to export a stored value under a different environment variable name.

### Docs

- Documented the `shine-env-export` helper and aliased env export workflow in the English and Chinese READMEs.

## [0.34.1] — 2026-06-26

### Bug Fixes

- Fixed JetBrains `.ideavimrc` `g` mappings to use consistent prefix across all configurations.
- Corrected `g/` mapping in JetBrains `.ideavimrc` that was previously mismatched.
- Upgraded `quinn-proto` dependency from 0.11.14 to 0.11.15 to resolve RUSTSEC-2026-0185 (remote memory exhaustion via unbounded out-of-order stream reassembly, severity 7.5).

## [0.34.0] — 2026-06-21

### Features

- Added OSC 11 terminal background detection to the managed Ubuntu and macOS sys profiles so interactive shells can keep bat's theme aligned with the active terminal.
- Added a bundled Shine Light theme for Ghostty and made it the default light appearance theme.

### Bug Fixes

- Fixed sys profile updates so UTF-8 BOM markers remain at the start of PowerShell profiles and profiles damaged by older versions are repaired automatically.
- Fixed sys profile installation to fall back to embedded templates when extracted external presets are stale.
- Added the pnpm global bin directory to the managed macOS sys profile PATH.
- Made release update checks tolerate transient failures and cache GitHub rate-limit responses to avoid repeated requests.

### Docs

- Documented terminal background theme synchronization and the bundled Ghostty Shine Light theme.

## [0.33.0] — 2026-06-14

### Features

- Added `shine completions install` for automatic `shine` command completion setup without installing presets, including dynamic preset category candidates.
- Added personal presets overlays so selected files can override embedded presets without switching to a full external presets directory.
- Split `shine sys init` shell integration into `pre` and `post` managed profile loaders so PATH and completion setup can run before user profile customizations while prompts and plugins stay near the end.
- Added `shine sys status` and sys init run tracking so previously initialized items can be inspected later.
- Added Rust and mise setup steps to the macOS sys init preset.
- Refreshed the bundled JetBrains IdeaVim preset with smartcase search, timeout tuning, and improved navigation mappings.

### Bug Fixes

- Fixed zsh completion setup so it initializes `compinit` when needed before registering `shine` completions.

### Docs

- Documented automatic shell completion setup, specific shell preset install refresh behavior, and the manual `shine completions <shell>` fallback.
- Documented presets overlays, `shine sys status`, updated macOS sys init items, and the `shine sys init` pre/post managed profile loader behavior.

## [0.32.0] — 2026-06-13

### Features

- Added `shine env export <KEY>` for shell-safe export code that decrypts `<KEY>_SECRET` when present and otherwise falls back to plaintext `<KEY>`.
- Added `shine env delete <KEY>` for removing stored env values from the active config.
- Improved `shine env encrypt --from <KEY>` so it stores encrypted output as `<KEY>_SECRET` by default, with `--set` still available for explicit targets.
- Added `gpg_key_id` as a default GPG recipient for `shine env encrypt`, with `-r/--recipient` available for per-command overrides.

### Docs

- Documented the new env secret export, delete, inferred encrypt target, and default GPG recipient workflows.

## [0.31.1] — 2026-06-11

### Bug Fixes

- Fixed update checks and self-upgrade requests to use the configured GitHub token and show GitHub API errors when release lookup fails.

### Docs

- Removed stale `shell/tools/test_tools` references from the English and Chinese README files and updated examples to the current `shell/utils/copyfile` preset.
- Updated the pinned `0.31.1` installer examples and version output examples in the README files.

## [0.31.0] — 2026-06-07

### Features

- Added top-level `shine install <CATEGORY>`, `shine reinstall <CATEGORY>`, and `shine uninstall <CATEGORY>` shims that resolve matching shell or app preset categories automatically.
- Added a `utils/copyfile` shell preset for copying a file's contents to the local clipboard via OSC52.
- Reworked system init profile handling so managed shell profile updates are merged from Rust while preserving user edits.

### Bug Fixes

- Fixed project-local config discovery so global `~/.shine/config.toml` settings do not override nearby `shine.config.toml` files.
- Fixed system init profile handling to preserve user customizations, accept uncommented default lines, and include Homebrew's zsh completions path.
- Clarified shell preset file counts in list output.

### Docs

- Documented the top-level install/reinstall/uninstall shims, the `copyfile` shell preset, and system init profile merge behavior.
- Updated the pinned `0.31.0` installer examples and version output examples in the README files.

## [0.30.0] — 2026-06-05

### Features

- Standardized summary and detail output across shell, app, list, and upgrade commands for more consistent CLI status rendering.
- Improved shell install and upgrade conflict reporting so blocked bin links show the existing entry, requested source, and a targeted reinstall command.

### Docs

- Removed stale fish shell support claims from the English and Chinese READMEs.
- Updated the pinned `0.30.0` installer examples and version output examples in the README files.

---

## [0.29.1] — 2026-06-04

### Bug Fixes

- Fixed shell installs from external presets so source-based commands are installed with the managed wrapper flow instead of being skipped or linked incorrectly.
- Fixed shell category uninstall so managed source wrappers are pruned along with the rest of the category.

### Docs

- Updated the pinned `0.29.1` installer examples and version output examples in the README.

---

## [0.29.0] — 2026-06-04

### Features

- Rendered `shine sys init` progress in Rust with compact per-item status rows, indented logs, summaries, and a lightweight status event protocol for sys preset scripts.

### Bug Fixes

- Fixed Ubuntu sys init Atuin profile initialization by loading the Atuin environment before running `atuin init`.
- Made sys init profile finalization report unchanged shell profiles as skipped instead of updated.
- Normalized Windows PowerShell shim paths so sourced commands such as `usetproxy` do not use `\\?\` verbatim paths.
- Improved sys init status alignment for long item names and trimmed empty version suffixes from status details.

### Docs

- Updated English and Chinese documentation for the new sys init execution model, status event protocol, and pinned `0.29.0` installer examples.

---

## [0.28.0] — 2026-06-03

### Features

- Added a Windows system init preset with selectable Rust, terminal tool, network, and JavaScript runtime setup steps.
- Added managed shell profile loaders for Windows and Ubuntu system init, including Yazi shell integration.
- Added a GitHub Light Default Ghostty theme and refreshed bundled Ghostty theme background customization.

### Bug Fixes

- Fixed Windows Atuin installation and PowerShell profile source path normalization.
- Fixed Ubuntu shell profile initialization targeting.
- Added SOCKS proxy support to GitHub release update checks.

### Docs

- Updated English and Chinese documentation for Windows system init support, managed profile setup, Ghostty themes, and the pinned `0.28.0` installer examples.

---

## [0.27.0] — 2026-06-01

### Features

- Added platform-scoped app preset metadata as a supported release feature, including platform-specific destination roots and file-level platform filtering.
- Split the bundled Docker app presets into `docker-engine` for Docker Engine daemon config and `docker-desktop` for Docker Desktop proxy settings.
- Added managed JSON key merging for app presets, with `docker-desktop` using it to update only the `proxy` and `containersProxy` keys in Docker Desktop `settings-store.json`.
- Added an implemented macOS system init preset with selectable Homebrew, terminal tool, editor, network, and JavaScript runtime setup steps.
- Expanded the Ubuntu system init preset with selectable Starship, zoxide, zsh-vi-mode, fzf, bat, eza, pnpm, mise, Homebrew, and ZeroTier setup steps.
- Moved sourced shell wrappers into the managed profile flow so helper functions are installed consistently with other shell presets.
- Refreshed the bundled Starship prompt preset with a more complete prompt layout and settings.

### Bug Fixes

- Avoided duplicate macOS zshrc setup during repeated system initialization.
- Avoided implicitly installing Homebrew when applying the Ubuntu recommended system init profile.
- Improved path display normalization and uninstall matching for app presets.

### Docs

- Documented the `docker-engine` / `docker-desktop` split, the Docker Engine vs Docker Desktop config-path distinction, and the new app-level `json-merge` install mode in the English and Chinese READMEs.
- Updated system init documentation for the implemented macOS preset and expanded Ubuntu tool set.

---

## [0.26.0] — 2026-05-30

### Features

- Added Windows PowerShell shell preset support, including native `setproxy` and `usetproxy` commands.
- Added a Windows `install.ps1` one-line installer for release assets.
- Added `shine upgrade --prune-stale` to remove managed app files whose preset source no longer exists.
- Added platform-specific proxy presets so PowerShell users get native `setproxy` and `usetproxy` scripts.
- Added a Windows PowerShell version of the `agent/ccenv` shell preset, so Claude Code provider setup now works in sourced PowerShell sessions as well as Unix shells.

### Bug Fixes

- Changed `setproxy` to keep Git, npm, and pnpm proxy behavior scoped to the current terminal session where possible; Yarn remains explicitly reported as a persistent config exception.
- Updated PowerShell PATH installation to write both supported profile locations on Windows, keeping `powershell.exe` and `pwsh.exe` in sync.

### Docs

- Documented the Windows installer, PowerShell proxy behavior, and session-scoped proxy defaults in the English and Chinese READMEs.
- Documented the Windows `ccenv` preset behavior, platform-scoped shell metadata entries, and dual PowerShell profile updates in the English and Chinese READMEs.

---

## [0.25.0] — 2026-05-20

### Features

- Added colorized status values in `shine info` output to make installed-state scans easier.
- Added expected-content diffs to `shine info` so drift is easier to inspect before reinstalling or upgrading.

### Docs

- Added a Chinese README and refreshed the `shine info` release examples in both READMEs.

---

## [0.24.0] — 2026-05-17

### Features

- Replaced forced preset installs with explicit `shine shell reinstall` and `shine app reinstall` commands.
- Added an IdeaVim keybind for the JetBrains "Find in Path" action.

---

## [0.23.1] — 2026-05-16

### Bug Fixes

- Fixed incorrect quick terminal keybind in the bundled Ghostty app preset.
- Binary installation now reports a permission error instead of silently ignoring it.

### Internal

- Various code quality improvements: stable content hashing, path-traversal validation, dead code removal, and refactoring of duplicate shell template logic.

---

## [0.23.0] — 2026-05-12

### Features

- Changed `shine info <target>` to show metadata and status by default, with `--verbose` preserving the previous full content output.

### Bug Fixes

- Collapsed the bundled Ghostty app preset to a single `ghostty` row in list and status output while still installing its config and theme files.

### Docs

- Updated README examples for the new `shine info --verbose` behavior and the aggregated Ghostty app listing.

---

## [0.22.0] — 2026-05-12

### Features

- Expanded the bundled `ghostty` app preset with paired `shine-light` and `shine-dark` themes, including optional background-image templating via `shine env`.
- Refined the default Ghostty preset styling by switching the shipped theme palette to Solarized and Alien Blood inspired variants.

### Bug Fixes

- Clarified `shine self upgrade --channel preview` status messaging when replacing a stable install with a preview build.

### Docs

- Updated the README release examples and Ghostty preset documentation for the new version and bundled theme files.

---

## [0.21.4] — 2026-05-10

### Bug Fixes

- Fixed `shine self upgrade --channel preview` so it no longer reinstalls the binary when the installed preview build already matches the current `+preview.<shortsha>` version.

### Docs

- Updated the preview self-upgrade documentation to clarify that matching preview builds are treated as up to date.

---

## [0.21.3] — 2026-05-09

### Features

- Added preset-driven `shine sys init` selection. System init presets can now define selectable items and named profiles in `presets/sys/<os>/shine.toml`, `shine sys init` offers an interactive multi-select in TTY sessions, and `shine sys init --preset <profile>` supports the same flow for scripts and automation.

### Bug Fixes

- Fixed a deadlock edge case where a failed `shine self upgrade` (e.g. GitHub API unreachable) would leave the update cache intact, causing every subsequent command to be permanently blocked by the "newer patch release required" gate until the network recovered. The cache is now cleared on upgrade failure so the next invocation either re-checks live or proceeds silently when the network is still down.

---

## [0.21.2] — 2026-05-09

### Features

- Renamed the top-level installed-content inspection command from `shine show <target>` to `shine info <target>`.

### Docs

- Updated the README examples and usage text to use `shine info`.

---

## [0.21.1] — 2026-05-09

### Fixes

- Read global `~/.shine/shine.env.toml` overrides even when no external `presets_dir` is configured, while keeping `~/.shine/config.toml` as the global config filename.

---

## [0.21.0] — 2026-05-08

### Features

- Added `shine env decrypt <KEY>`, so presets and shell helpers can decrypt base64-encoded GPG secrets from the active env config at runtime instead of duplicating decryption logic.
- Added `shine env encrypt --recipient <key-id>` to generate reusable base64-encoded GPG secrets from stdin, with `--from <KEY>` and `--set <KEY>` for encrypting and storing active `[env]` values directly.
- Added stable and preview self-upgrade channels, including `shine self upgrade --channel preview` for installing the moving `preview` prerelease and `--channel stable` for explicitly reinstalling the latest stable release.
- Marked preview binaries at build time so `shine --version` reports build metadata such as `0.21.0+preview.<shortsha>` while stable builds remain `0.21.0`.

### Internal

- Added a dedicated preview release packaging workflow that publishes fixed-name `shine-preview-{target}.tar.gz` assets from the release branch and injects `SHINE_VERSION_METADATA=preview.${GITHUB_SHA::7}` without changing stable archive names or update ordering.

### Docs

- Refreshed the pinned `install.sh` example to `0.21.0`.
- Documented that preview binaries report `+preview.<shortsha>` in `shine --version` while stable binaries keep the plain release version.
- Translated bundled preset comments and helper text from Chinese to English for the Vim, proxy, and tools presets.

---

## [0.20.0] — 2026-05-08

### Features

- Added an `agent` shell preset with `ccenv`, which configures Claude Code to use the DeepSeek provider from `shine.env.toml`.
- Added metadata for source-required shell helpers, so commands like `ccenv` can be exposed with clearer installed names and usage expectations.
- Added support for `DEEPSEEK_API_KEY_GPG_SECRET`, a base64-encoded GPG secret for `ccenv` that can be decrypted through reusable `shine env decrypt` GPG/YubiKey support at runtime.

### Fixes

- Clarified proxy helper activation by detecting direct execution and instructing users to source `setproxy` and `usetproxy` when needed.
- Updated the Claude Code helper to read `DEEPSEEK_API_KEY` from the active env config.

### Docs

- Documented the new `agent` preset, updated shell preset examples, and refreshed the pinned `install.sh` example to `0.20.0`.

---

## [0.19.0] — 2026-05-07

### Features

- Renamed project-local configuration to `shine.config.toml` and project env overrides to `shine.env.toml`, avoiding collisions with other tools' `config.toml` and `.env.toml` files.
- Made project config discovery walk up ancestor directories and resolve relative `presets_dir` values from the discovered config directory, so cloned presets repos work from the repo root or subdirectories.

### Docs

- Documented the new project config filenames, legacy compatibility behavior, and the pinned `install.sh` example for `0.19.0`.

## [0.18.0] — 2026-05-06

### Features

- Added `shine init` to initialize the current directory as a project-local presets source.
- Added current-directory config discovery and project env overrides for project-local preset repositories.

### Fixes

- Hid shell presets from `shine list` when the preset source exists but the managed bin symlink is missing, so the installed-only view only shows callable shell commands.

### Docs

- Documented project-local initialization and the refined `shine list` behavior.
- Updated the pinned `install.sh` example to `0.18.0`.

## [0.17.0] — 2026-05-05

### Features

- Changed `shine update` to show only available shell, app, and self updates by default, with `--verbose` preserving the full installed status view.
- Aligned shell preset status wording with app configs by showing installed shell presets as `up-to-date`.

### Docs

- Updated README examples for the focused `shine update` output and refreshed the pinned install example to `0.17.0`.

## [0.16.1] — 2026-05-05

### Fixes

- Fixed `shine self upgrade` so syncing a remembered `shine self install` destination copies from the newly installed binary path instead of the deleted backup path left behind by the running process.

## [0.16.0] — 2026-05-05

### Features

- Added `shine completions <shell>` for `bash`, `zsh`, and `fish`, so shell completion scripts can be generated for manual installation.
- Refreshed the bundled Ghostty preset with updated theme and background settings.

### Docs

- Documented shell completion generation and installation examples.
- Clarified that `shine` currently supports Unix-like environments with `bash`, `zsh`, and `fish`, and does not support Windows, PowerShell, or Elvish yet.
- Updated the pinned `install.sh` example to `0.16.0`.

### Fixes

- Added a top-level help description for `shine completions`, so the command is discoverable from `shine --help`.

## [0.15.1] — 2026-05-05

### Fixes

- Fixed `shine self upgrade` so the remembered `shine self install` destination is synced atomically and recreated when its parent directory is missing, avoiding a noisy warning for stale `/usr/local/bin/shine` paths.

## [0.15.0] — 2026-05-05

### Features

- Added `shine show <target>` to inspect installed app configs and shell presets, including metadata and full installed file or effective script content.

### Docs

- Documented `shine show` usage and updated the pinned install example to `0.15.0`.

### Fixes

- Fixed `shine upgrade` so external shell presets only upgrade commands that are already installed, preventing preset-only scripts such as `tools/test_tools` from being installed unexpectedly.
- Made shell template rendering fail fast when `~/.shine/rendered` cannot be written, instead of continuing and linking raw template scripts.
- Reduced default `shine upgrade` noise by hiding skipped app config rows while still counting them in the final summary.
- Reported shell template updates under **Shell Presets** and app template updates under **App Configs**, so entries such as `proxy/setproxy` and `docker/daemon.jsonc` appear in the section that owns the change.

## [0.14.5] — 2026-05-04

### Fixes

- Fixed `shine upgrade` so app configs that are already up to date, such as `app/docker/daemon.jsonc`, are skipped instead of rewritten and counted as updated.
- Preserved user-modified app config destinations during upgrade by skipping them instead of force-overwriting managed files.

## [0.14.4] — 2026-05-04

### Fixes

- Fixed `shine upgrade` for existing proxy installs so stale shell config blocks are refreshed with the `setproxy` / `usetproxy` source wrapper functions. Upgraded installs can use `setproxy` directly again without manually prefixing `source`.

## [0.14.3] — 2026-05-04

### Fixes

- Reduced `shine upgrade` output noise by printing a single external-presets note and one final summary.
- Added `shine upgrade --verbose` for expanded env-template checks while keeping the default output focused on actionable changes.
- Suppressed shell PATH status during `shine upgrade` when the shell config is already correctly configured.

## [0.14.2] — 2026-05-04

### Features

- Simplified external preset management commands: `shine export`, `shine link`, and `shine unlink` are now top-level commands, and the old `shine presets ...` entrypoints were removed.

## [0.14.1] — 2026-05-04

### Features

- Added bundled **Ghostty** app preset. `shine app install ghostty` now installs `config.ghostty` to `~/.config/ghostty/config.ghostty`.

### Docs

- Updated README app preset examples to include Ghostty and refreshed the documented preset layout.
- Added repo-local verification notes for sandboxed `shine` CLI checks, including `--target-dir target` and `SHINE_CONFIG_DIR` caveats.

## [0.14.0] — 2026-05-03

### Features

- **Command update flow refactor** — `shine self upgrade` now handles binary upgrades, while top-level `shine upgrade` force-updates installed shell and app configs.
- **`shine update` status preflight** — manual update checks now show installed config status before checking the latest release.
- **Simplified `shine list`** — the installed-only list now shows only configured items without status labels.
- **Env config moved into `config.toml`** — template variables now live under `[env]` in `~/.shine/config.toml`. Existing `env.toml` files are migrated automatically and removed after a successful migration.
- **Ubuntu `shine sys init` now installs Yazi** — the bundled Ubuntu system init preset now installs Yazi from the latest official release, pulls in the required preview/runtime dependencies, and creates an `fd` compatibility symlink for Debian-based systems that ship `fdfind`.

### Breaking Changes

- Removed the public `shine env upgrade` command. After changing env values, run `shine upgrade` to apply them to installed presets.
- Removed the public `shine env path` command because env values now live in `config.toml`.
- Removed the public `shine check` command. Use `shine update` for installed configuration status.

### Fixes

- **External shell template update detection** — `shine update` now reports updates for installed shell scripts when an external `presets_dir` template, such as `shell/proxy/set_proxy.sh`, changes and needs to be re-rendered with the current `[env]` values.
- **`shine self install` overwrite behavior** — installing from a newer binary now stages to a temporary file and atomically replaces the old destination. Running the already-installed system copy now fails with an actionable message instead of pretending to reinstall itself.

---

## [0.13.3] — 2026-05-03

### Fixes

- **`setproxy` / `usetproxy` now work without `source` prefix** — running `setproxy` directly in a terminal no longer silently drops environment variables. `shine shell install` now writes shell wrapper functions (`setproxy() { source ... }`) into the shell config sentinel block for any preset declared with `needs_source = true`, so proxy env vars are properly exported to the calling shell. The `proxy` category presets carry this flag automatically.

---

## [0.13.2] — 2026-05-03

### Features

- **`shine sys list` and `shine sys init`** — new `sys` subcommand group for system-level initialization. `shine sys list` enumerates available OS init presets and marks the current platform with a ▶ indicator. `shine sys init [--dry-run]` detects the running OS and executes the corresponding script from `presets/sys/<os>/init.sh`.

- **Ubuntu init preset** — idempotent bootstrap script that installs Neovim (from GitHub Releases tarball to guarantee v0.10+ — Ubuntu 22.04 apt only ships 0.6.x), AstroNvim, and Atuin (via the official installer). macOS has a placeholder stub.

- **Proxy host and no-proxy configurable via `env.toml`** — `PROXY_HOST` (default `127.0.0.1`) and `PROXY_NO_PROXY` (default `localhost,127.0.0.1,::1`) are now seeded into `env.toml` on first run and backfilled automatically into existing files on upgrade. Both `presets/app/docker/daemon.jsonc` and `presets/shell/proxy/set_proxy.sh` now use these variables, so changing the proxy host once in `env.toml` and running `shine env upgrade` updates all installed files.

- **`shine env upgrade` processes shell scripts** — previously only app manifest entries were re-rendered; shell scripts that declare `# shine-template: true` are now also processed. Rendered outputs are written to `~/.shine/rendered/shell/` so user-owned templates in `presets/` are never modified.

- **Rendered shell scripts isolated from presets** — shell scripts with template substitution are now rendered to `~/.shine/rendered/shell/<category>/` rather than being written back into the presets directory. This ensures external-preset users (after `shine presets export`) keep clean, editable templates. Existing bin symlinks that still point to the old presets location are migrated transparently during `shine env upgrade`.

### Fixes

- **`shine shell uninstall`** — now removes symlinks pointing to both the legacy presets location and the new rendered location, so no stale links are left after uninstall.

- **CI** — `open-main-pr` workflow job now handles PR creation permission errors gracefully instead of failing the entire workflow.

---

## [0.13.1] — 2026-05-02

### Fixes

- **`shine update` / `shine upgrade`** — update checks no longer fail just because `~/.shine/update-check.json` cannot be written. Shine now recreates the cache directory when needed, and treats cache persistence as best-effort after a successful GitHub release check.

## [0.13.0] — 2026-05-02

### Features

- **Shell preset rename support** — shell categories may now define `presets/shell/<category>/shine.toml` with `[[files]]` entries using `source` and optional `target`, so installed command names no longer have to match script filenames

- **Proxy commands renamed** — the bundled proxy preset now installs `setproxy` and `usetproxy` as the public shell commands while keeping the underlying script files as `set_proxy.sh` and `uset_proxy.sh`

### Docs

- Updated README examples and directory layout to show the new proxy command names and document shell preset `shine.toml` rename metadata

## [0.12.0] — 2026-05-01

### Features

- **`~/.shine/env.toml`** — new user-editable environment config file, seeded on first run with `HTTP_PROXY_PORT = "6152"` and `SOCKS5_PROXY_PORT = "6153"`. Values are substituted into preset files that use the new `template` transform (`@@VAR_NAME@@` placeholder syntax).

- **`shine env` subcommand** — manage env variables:
  - `shine env show` — list all variables and their values
  - `shine env set KEY VALUE` — set a variable, preserving existing comments
  - `shine env get KEY` — print a single variable value
  - `shine env path` — print the path of env.toml

- **`shine env upgrade`** — re-render all installed preset files that used the `template` transform with the current env values. Detects user-modified destinations (skips them with a warning) and supports `--dry-run`.

- **`template` transform** — new transform step for app presets (`transforms = ["template", "jsonc-to-json"]`). Replaces `@@VAR_NAME@@` placeholders from `env.toml`. Errors on undefined variables (all missing names reported at once).

- **Shell preset template support** — shell scripts may opt into substitution by adding `# shine-template: true` after the shebang. The `proxy/set_proxy.sh` preset now uses `@@HTTP_PROXY_PORT@@` and `@@SOCKS5_PROXY_PORT@@` so ports are driven by `env.toml`.

- **Docker preset updated** — `daemon.jsonc` now uses `@@HTTP_PROXY_PORT@@`; the `shine.toml` chains `["template", "jsonc-to-json"]` transforms so the installed `daemon.json` reflects the configured port automatically.

---

## [0.11.5] — 2026-05-01

### Fixes

- **`sudo shine app install` / `sudo shine presets link`** — config file and all paths containing `~` now resolve to the invoking user's home directory instead of `/root`. `sudo` resets `HOME` to `/root`; shine now detects `SUDO_USER` and looks up the real home from `/etc/passwd`. Affected: config file location, `presets_dir`, `app_default_dest_root`, `SHINE_CONFIG_DIR`, `SHINE_PRESETS`, and every destination path expanded by `shellexpand`

---

## [0.11.4] — 2026-05-01

### Fixes

- **`shine self install`** — permission-denied error now shows the full binary path in the hint (e.g. `sudo /home/felix/.local/bin/shine self install`) instead of the bare `shine` name that sudo cannot resolve

---

## [0.11.3] — 2026-05-01

### Features

- **`shine self install [--dest <PATH>]`** — copies the current binary to `/usr/local/bin/shine` (or a custom path) so `sudo shine` resolves correctly without specifying the full path

### Fixes

- **`shine check` / `shine list` / `shine presets`** — commands were silently unregistered in the debug binary due to the `Commands` enum deriving `Parser` instead of `Subcommand`; corrected to `#[derive(Subcommand)]`

---

## [0.11.2] — 2026-05-01

### Maintenance

- Upgrade **reqwest** `0.12.24 → 0.13.3` (TLS feature renamed `rustls-tls` → `rustls`)
- Upgrade **jsonc-parser** `0.26 → 0.32.3` (API: `parse_to_serde_value` now generic, returns `T` directly instead of `Option<T>`)

---

## [0.11.1] — 2026-05-01

### Fixes

- **`shine app uninstall`** no longer deletes files from a user-managed external `presets_dir`; preset cleanup is now skipped when `is_external_presets` is set
- **`shine presets export`** no longer prints the `shine presets link` tip when `presets_dir` is already configured in `config.toml` or via `SHINE_PRESETS`

---

## [0.11.0] — 2026-04-30

### Features

**File transforms in `shine.toml` — convert files during install**
- Declare a `transform` (or `transforms` pipeline) on any `[[files]]` entry to process a source file before it is written to its destination
- First supported transform: `jsonc-to-json` — strips `//` line comments, `/* */` block comments, and trailing commas from a JSONC file and writes valid JSON to the target path
- Combine with `target` to rename the file at the destination (e.g. `daemon.jsonc` → `daemon.json`)
- `shine check` compares the transformed output against the installed file, so editing a comment-only line in the source JSONC that produces identical JSON is correctly reported as **up-to-date** rather than an available update
- Install output annotates transform steps: `✓  daemon.jsonc  [jsonc-to-json]  →  /etc/docker/daemon.json`
- Invalid transform names fail at preset load time with a clear error, not mid-install
- Built-in docker preset updated to use the new mechanism (`daemon.jsonc → daemon.json`)

---

## [0.10.0] — 2026-04-30

### Features

**External presets — manage your own preset files outside the binary**
- Configure a custom `presets_dir` in `~/.shine/config.toml` (or via `SHINE_PRESETS`) to load shell scripts and app configs from the filesystem instead of the embedded binary
- `shine presets export` copies all built-in presets to `presets_dir`, giving you a starting point to customize
- `shine shell install` / `shine app install` install directly from the external directory when it is configured; the binary's embedded presets are bypassed entirely
- `shine check` and `shine list` reflect status against the external source: `UpdateAvail` is computed by comparing installed files against the filesystem copy rather than the embedded asset
- Command output annotates which preset source is active (external path shown in **bold cyan** when `is_external_presets` is set)

**Improved partial-category status in `shine check`**
- When a category has some files installed and some missing, `UpdateAvail` and `UserModified` now take priority over `Partial` so the most actionable status is surfaced; `Partial` is shown only when all installed files are otherwise up-to-date

---

## [0.9.0] — 2026-04-29

### Features

**`shine list` — show installed items at a glance**
- New top-level command that prints only installed shell presets and app configs, filtered from `shine check` output
- Displays two aligned sections (Shell Presets, App Configs) with the same status symbols as `shine check`
- Shows a compact summary footer; prints a helpful hint when nothing is installed yet

**`shine shell uninstall [CATEGORY]` — per-category shell uninstall**
- Optional positional `CATEGORY` argument scopes removal to a single preset category (e.g. `shine shell uninstall proxy`)
- Only that category's preset files and bin symlinks are removed; the PATH sentinel is preserved so other installed categories remain usable
- `--purge` with a category removes only that category's subdirectory; without a category the existing full-cleanup behaviour is unchanged
- Omitting the argument keeps the existing all-categories behaviour

**`shine app uninstall [CATEGORY]` — per-category app uninstall**
- Same optional `CATEGORY` argument for app configs (e.g. `shine app uninstall starship`)
- Uninstalls only that category's managed files and restores any `.shine.bak` backups; `--purge` removes only the category's presets subdirectory
- Omitting the argument keeps the existing all-categories behaviour

---

## [0.8.0] — 2026-04-29

### UX

**Terminal output beautification across all commands**
- `shine check`: bold section headers, aligned label columns, colored status text, dim paths with `→` arrow, Summary line uses `·` separator with per-status colors
- `shine app list`: name-aligned layout, dim file counts and hint text
- `shine app install/uninstall`: dim paths and arrows, unified **Done** summary with colored `·` separated counts
- `shine shell install/uninstall/list`: bold section headers, colored created/skipped/removed counts
- Added `bold()`, `dim()`, `cyan()`, `status_label()` helpers to `colors.rs`; all output degrades gracefully to plain text when stdout is not a TTY or `NO_COLOR` is set

---

## [0.7.0] — 2026-04-29

### Features

**`shine app info <CATEGORY>`**
- New subcommand that prints the description, destination, and file list for a single app category
- Shows `display_name`, source, target, and per-file description when available

**`shine app list` — improved output**
- Beautified layout with aligned columns
- Simplified to show only essential information

**`shine check` — per-file rows for explicit `[[files]]` categories**
- Categories that declare an explicit `[[files]]` section in `shine.toml` now emit one status row per file instead of a single aggregated category row
- Row label uses the new `display_name` field when set (e.g. `JetBrains/IdeaVim`), falling back to `{category}/{source}`
- Legacy and auto-collected categories keep the existing single-row aggregated behavior

### Presets

- Added `shine.toml` with `dest` for **archey4** and **fastfetch** categories
- **JetBrains**: migrated to explicit `[[files]]` declaration; removed `shine-dest` annotation from `.ideavimrc`; added `display_name = "JetBrains/IdeaVim"`

### Schema

- `shine.toml` `[[files]]` entries now support an optional `display_name` field to control the label shown in `shine check` output

---

## [0.6.1] — 2026-04-28

### UX

- ANSI colors are now applied consistently across all status-bearing output
- Added shared `colors` module (`✓` green, `↑` cyan, `~` yellow, `!` magenta, `✗` red)
- `shine app install` / `uninstall` — file-level status lines are now colored
- `shine update` / `upgrade` — result messages colored (success → green, warning → yellow)
- Colors degrade automatically to plain text when stdout is not a TTY or `NO_COLOR` is set

---

## [0.6.0] — 2026-04-27

### Features

**`shine check` — local config audit**
- Added `shine check` to display which app configs and shell presets are applied locally
- `shine check app` — one status line per app category with aggregated status across all files in that category
- `shine check shell` — per-script install status (preset file + bin symlink) plus PATH sentinel detection
- `shine check` with no subcommand shows both shell and app status

App status symbols:
- `✓` all files up-to-date
- `↑` shine has a newer version — run `shine app install`
- `~` user-modified or partial install
- `!` destination file missing (was installed, now deleted)
- `✗` not installed

Multi-file categories (e.g. `vim` with `dest = "~/.vim"`) are reported as a single unit

---

## [0.5.1] — 2026-04-27

### Features

- App preset categories now support a `shine.toml` manifest declaring `dest`, optional per-file `target` overrides, and `description` fields
- When `shine.toml` is absent the legacy `shine-dest:` annotation and default-root fallback are still used (backwards compatible)
- Added bundled vim preset with `shine.toml` (`presets/app/vim/`)

---

## [0.5.0] — 2026-04-25

### Features

**App preset management**
- Added `shine app list`, `shine app install`, and `shine app uninstall` for managing non-shell configuration presets
- App categories can now declare `presets/app/<category>/shine.toml` for directory-level install targets such as `vim -> ~/.vim`
- `shine.toml` supports both explicit file lists and whole-directory mapping when `files` is omitted
- App presets can declare a `shine-dest:` annotation for explicit install targets such as `~/.gitconfig`, `~/.ideavimrc`, or `~/.config/starship/starship.toml`
- Presets without an annotation now install under `app_default_dest_root/<CATEGORY>/<FILE>`, with `~/.config` used by default
- Existing unmanaged destination files are backed up to `*.shine.bak` before install, and matching backups are restored during uninstall
- Installed app files are tracked in `~/.shine/app-manifest.toml` so managed updates and removals stay deterministic

### Docs

- README now documents the new `shine app` workflow, destination resolution rules, backup behavior, and current bundled app presets
- README pinned-version install example updated to `0.5.0`

### Internal

- Added app preset fixtures for JetBrains IdeaVim, git, and starship

---

## [0.4.1] — 2026-04-25

### Fixes

- `install.sh` now resolves the actual latest GitHub release tag before building the asset download URL, so `SHINE_VERSION=latest` installs correctly
- `shine update` and version-gate failures now print clearer user-facing messages with proper exit handling

### Docs

- README pinned-version install example updated to `0.4.1`

### Internal

- Added `renovate.json` to automate dependency update management
- Added `CDLA-Permissive-2.0` to the cargo-deny license allowlist

---

## [0.4.0] — 2026-04-24

### Features

**GitHub Release self-upgrade**
- Added `shine upgrade` to download and install the latest GitHub Release asset for the current macOS/Linux platform
- Upgrade installs the matching `darwin`/`linux` and `x86_64`/`aarch64` asset, extracts the packaged binary, and replaces the current executable in place
- Successful upgrades refresh the local update-check cache so subsequent commands do not keep warning about the old version

**Update command coexistence**
- Kept `shine update` as the manual version-check command while adding `shine upgrade` as the install action
- Runtime update warnings now direct users to run `shine upgrade` when a newer release is available

**Release installer script**
- Added top-level `install.sh` for one-step installation from GitHub Releases into `~/.local/bin`
- Supports `SHINE_INSTALL_DIR`, `SHINE_VERSION`, and `SHINE_REPO` overrides for custom install locations, pinned versions, or alternate repositories
- Detects the current platform, downloads the matching `tar.gz` asset, installs `shine`, and warns when the install directory is not on `PATH`

**Release asset publishing**
- GitHub Actions now builds versioned Release assets for `darwin-x86_64`, `darwin-aarch64`, `linux-x86_64`, and `linux-aarch64`
- Tag builds upload packaged `shine-v{version}-{target}.tar.gz` archives together with `install.sh` to the GitHub Release

### Docs

- README now documents `shine update` vs `shine upgrade`, GitHub Release installation, and `install.sh` environment variables

### Internal

- Added release-asset selection and archive extraction tests for the new upgrade flow
- Stabilized config tests that mutate `SHINE_CONFIG_DIR` and `SHINE_PRESETS` under parallel test execution

---

## [0.3.2] — 2026-04-24

### Features

**Manual update check command**
- Added `shine update` command to manually trigger a version check against the latest GitHub Release
- Bypasses the 24-hour local cache, always fetches the current release from GitHub
- Prints the installed version alongside the latest; exits with an error if a required patch update is pending
- Other commands continue to use the cached check (no extra network round-trip)

### Fixes

- Added a 5-second timeout to the GitHub release HTTP request to prevent indefinite hangs on slow or unreachable networks

---

## [0.3.1] — 2026-04-24

### Features

**Suffix-free installed commands**
- Installed shell commands are now accessible without the `.sh` extension (e.g. `set_proxy` instead of `set_proxy.sh`)
- `~/.shine/bin/` symlinks now use the file stem; known extensions stripped: `.sh`, `.bash`, `.zsh`, `.fish`, `.ps1`
- Collision detection uses the stem, so `foo.sh` and `foo.zsh` in the same category correctly report a conflict

### Docs

- `shine shell list` footer now states that commands are available directly by name after installation
- Usage hints in all bundled preset scripts updated to omit `.sh` suffix

---

## [0.3.0] — 2026-04-24

### Features

**Runtime release update check**
- `shine` now checks the latest GitHub Release for `biulight/shine` before executing commands
- Latest release lookup is cached locally for 24 hours under the shine config directory
- Version comparison follows SemVer semantics
- Newer `major` or `minor` versions show an upgrade reminder and continue execution
- Newer `patch` versions require the user to upgrade before the command continues
- Network errors, API failures, and invalid cache state are ignored so normal commands still run

**Unified CLI versioning**
- The CLI version now reads from `[workspace.package].version`
- `shine --version` and the compiled package version stay aligned with the workspace release version

### Docs

- README now documents runtime update behavior
- README build output path corrected to `target/release/shine`

## [0.2.0] — 2026-04-23

### Features

**`shine shell list`** _(new command)_
- Lists all bundled preset categories grouped by subdirectory under `shell/`
- Displays per-script descriptions parsed from the leading comment block of each `.sh` file (lines starting with `# ` immediately after the shebang)
- Aligned two-column output: script name on the left, multi-line description on the right

**`shine shell install [CATEGORY]`** _(extended)_
- New optional `CATEGORY` positional argument; omitting it installs all shell presets (previous behavior)
- `shine shell install proxy` installs only `shell/proxy/` presets
- `--help` hints to run `shine shell list` to see available categories

**Auto PATH injection**
- `install` appends a sentinel-guarded PATH block to the detected shell config file (`~/.zshrc`, `~/.bashrc`, `~/.config/fish/config.fish`, etc.)
- Uses `$HOME`-relative path when `bin_dir` is under the home directory
- Bash/Zsh guard: `if [[ ":$PATH:" != *":$HOME/.shine/bin:"* ]]` prevents duplicate entries on re-source
- Fish: uses `fish_add_path` (idempotent by default)
- `uninstall` removes the sentinel block precisely; `--dry-run` leaves the config untouched
- Idempotent: a second `install` prints "already configured, skipped"

**New preset: `shell/tools/test_tools.sh`**
- Verifies that shine-installed shell tools are callable from the current environment

**Preset script comment headers**
- All bundled `.sh` scripts now carry a structured multi-line `# description` block immediately after the shebang, consumed by `shine shell list`

### Removed

- `shine shell proxy` standalone subcommand — superseded by `shine shell install proxy`

### Internal

- `cli/build.rs` added: registers `cargo:rerun-if-changed=../presets` so `rust-embed` recompiles when preset files are added or modified
- `presets::list_categories` and `presets::parse_script_description` public helpers
- `shells::path_export_snippet`, `append_path_to_shell_config`, `remove_path_from_shell_config` helpers
- Test count: 57 → 66

---

## [0.1.0] — 2026-04-23

Initial release of `shine`.

### Features

**`shine shell install`**
- Extract embedded shell preset scripts to `~/.shine/presets/shell/`
- Create `~/.shine/bin/` directory and populate it with flat symlinks to installed executable scripts
- Idempotent: existing correct symlinks and files are skipped on re-run
- Conflict detection: reports collisions without overwriting user files

**`shine shell uninstall`**
- Remove shine-managed symlinks from `~/.shine/bin/` (user-created symlinks with external targets are untouched)
- Remove embedded-asset preset files from `~/.shine/presets/shell/` (user-added files are untouched)
- `--dry-run` flag: print what would be removed without making any changes
- `--purge` flag: additionally remove empty managed directories (`bin/`, `presets/shell/`, `presets/`) after uninstall; never removes `config.toml`
- Fully idempotent: second run is a no-op

**Bundled presets: `shell/proxy`**
- `set_proxy.sh`: one-command proxy setup for system env, git, npm, yarn, pnpm
  - Auto mode: detects SOCKS5 availability, falls back to HTTP
  - Explicit modes: `auto`, `sock5`, `http`
  - Default ports: HTTP 6152, SOCKS5 6153
- `uset_proxy.sh`: one-command proxy teardown for all of the above

**Configuration**
- `~/.shine/config.toml` created automatically on first run
- TOML comment preservation on in-place updates (via `toml_edit`)
- `SHINE_CONFIG_DIR` environment variable overrides the default `~/.shine/` location
- `SHINE_PRESETS` environment variable overrides the presets directory only
- `presets_dir` key in `config.toml` as a persistent override

**Supported shells**
- bash, zsh, fish, powershell, elvish

### Internal

- Workspace: `cli` (binary) + `utils` (TOML migration library)
- 57 unit and integration tests
- Pre-commit hooks: `cargo fmt`, `cargo clippy -D warnings`, `cargo deny check`, `typos`, `cargo nextest`
