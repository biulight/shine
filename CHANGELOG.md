# Changelog

All notable changes to this project will be documented in this file.
See [Conventional Commits](https://www.conventionalcommits.org/) for commit guidelines.

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
