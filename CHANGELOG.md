# Changelog

All notable changes to this project will be documented in this file.
See [Conventional Commits](https://www.conventionalcommits.org/) for commit guidelines.

---

## [Unreleased] — 2026-04-23

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
