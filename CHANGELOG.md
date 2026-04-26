# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [0.5.1] — 2026-04-27

### Bug Fixes

- Fix clippy `cmp_owned` warnings: replace `PathBuf::from(...)` with `Path::new(...)` in equality comparisons (`apps/metadata.rs`, `apps/mod.rs`)

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
## [0.4.1] — 2026-04-24

### Bug Fixes

- `shine update` now resolves the `latest` tag via GitHub API before constructing the asset download URL, fixing update failures
- Improved update command messaging and added license exception handling

### Internal

- Added Renovate configuration for automated dependency updates

---
## [0.4.0] — 2026-04-24

### Features

- Added `shine install` / `shine update` commands to install or upgrade the binary from GitHub Releases

### Bug Fixes

- Switched reqwest TLS backend from `default-tls` to `rustls-tls` to fix ARM64 cross-compilation
- Dropped `macos-13` from release runner matrix

---
## [0.3.2] — 2026-04-24

- Patch release with no user-facing changes (CI/CD fixes only)

---
## [0.3.1] — 2026-04-24

### Features

- Installed shell commands are now symlinked without the `.sh` suffix so they run as plain commands (e.g. `set_proxy` instead of `set_proxy.sh`)

---
## [0.3.0] — 2026-04-23

### Features

- Added runtime update check: `shine` notifies the user when a newer version is available
- Registered `shine` as the binary name in `Cargo.toml`

---
## [0.2.0] — 2026-04-23

### Features

- `shine shell install` improvements: smarter PATH injection and shell config detection
- Added MIT license

### Docs

- Added README, CHANGELOG, and CLAUDE.md
