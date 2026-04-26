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
## [0.4.1](https://github.com/biulight/shine/compare/v0.4.0..v0.4.1) - 2026-04-24

### Bug Fixes

- **(cli)** improve update messaging and add license exception - ([94efa45](https://github.com/biulight/shine/commit/94efa45eee494cd908f46d744ff8b739a7f9b0d1)) - biulight
- **(install)** resolve 'latest' version tag from GitHub API before building asset URL - ([ded26b4](https://github.com/biulight/shine/commit/ded26b43c4362b790b69f22751786d33ccb58690)) - biulight

### Miscellaneous Chores

- add Renovate configuration for dependency management - ([a5531c0](https://github.com/biulight/shine/commit/a5531c03894113231265394fb8fdc3443cc501b4)) - biulight
- release 0.4.1 - ([2348a0e](https://github.com/biulight/shine/commit/2348a0e0d4db851be6a2d5a5403a6d71da4dfea9)) - biulight

---
## [0.4.0](https://github.com/biulight/shine/compare/v0.3.2..v0.4.0) - 2026-04-24

### Bug Fixes

- switch reqwest TLS backend from default-tls to rustls-tls for ARM64 cross-compilation - ([c70c2b9](https://github.com/biulight/shine/commit/c70c2b905b251e7873af96282bae0893c7e5f92d)) - copilot-swe-agent[bot]
- drop macos-13 release runner - ([1bc2733](https://github.com/biulight/shine/commit/1bc2733130840ed44a28d3c7ae0a9e9767bc9f5f)) - biulight

### Features

- add release install and upgrade flow - ([d981f2c](https://github.com/biulight/shine/commit/d981f2ce6c793cc1aa15999cb420e9303cf10645)) - biulight

### Miscellaneous Chores

- release 0.4.0 - ([fa9d2b0](https://github.com/biulight/shine/commit/fa9d2b0049153b7de06a0d50aec364c50eb98046)) - biulight

### Other

- Initial plan - ([5bcd1c6](https://github.com/biulight/shine/commit/5bcd1c60e223648f6fe17fa209803705c7e88502)) - copilot-swe-agent[bot]
- Merge pull request #2 from biulight/copilot/fix-github-actions-build-release-assets

Fix ARM64 cross-compilation: switch reqwest TLS backend from OpenSSL to Rustls - ([ad2c2d6](https://github.com/biulight/shine/commit/ad2c2d65eac97ba737b61dc18df3a297589e3d4f)) - Felix Jiang

---
## [0.3.2](https://github.com/biulight/shine/compare/v0.3.1..v0.3.2) - 2026-04-24

### Miscellaneous Chores

- release 0.3.2 - ([d225db2](https://github.com/biulight/shine/commit/d225db2836d9d2b407067bb6b9b212a30a2a878d)) - biulight

---
## [0.3.1](https://github.com/biulight/shine/compare/v0.3.0..v0.3.1) - 2026-04-24

### Features

- install shell commands without .sh suffix - ([03b6b32](https://github.com/biulight/shine/commit/03b6b32ef012b2525af75a9374c4f70199f2ac01)) - biulight

### Miscellaneous Chores

- release 0.3.1 - ([1ab77c7](https://github.com/biulight/shine/commit/1ab77c72a0ccabe22b2be7a55f9de6ae9ea962fa)) - biulight

---
## [0.3.0](https://github.com/biulight/shine/compare/v0.2.0..v0.3.0) - 2026-04-23

### Features

- **(cli)** add shine binary definition to Cargo.toml - ([e3736dc](https://github.com/biulight/shine/commit/e3736dc18ac47276821f5644b1da24ce0259dd92)) - biulight
- add runtime update check - ([4100aca](https://github.com/biulight/shine/commit/4100aca78e594a513cd73d5f0c71989a744750a7)) - biulight

### Miscellaneous Chores

- release 0.3.0 - ([3c7926f](https://github.com/biulight/shine/commit/3c7926f2c905bf9052d388b0466bb3fd0640d03a)) - biulight

---
## [0.2.0] - 2026-04-23

### Documentation

- update README, CHANGELOG, and add CLAUDE.md - ([7bbb752](https://github.com/biulight/shine/commit/7bbb7522fe586658e6a65a293c11bcb074cccae0)) - biulight

### Features

- initial release of shine v0.1.0 - ([8afbd3f](https://github.com/biulight/shine/commit/8afbd3f23dafc473d48410b79d1c42c9b3467517)) - biulight
- shell install improvements - ([e2723a0](https://github.com/biulight/shine/commit/e2723a0f160458ca87b41ccc77d2871f94d5034e)) - biulight

### Miscellaneous Chores

- Add MIT license - ([13177aa](https://github.com/biulight/shine/commit/13177aa28ba7bff11a9b4ccecea230f5f5306758)) - biulight
- bump version to 0.2.0 - ([cc5a4a0](https://github.com/biulight/shine/commit/cc5a4a0a9e60e391f8e3a7a3e22552b18d2058a0)) - biulight

### Other

- fix workflow — use main branch and upgrade git-cliff-action to v4 - ([090c1f1](https://github.com/biulight/shine/commit/090c1f1e9cdc1a40b6a2402c73fff75fb1ebbcd5)) - biulight

<!-- generated by git-cliff -->
