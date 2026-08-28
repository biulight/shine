---
title: Customize presets
sidebar_position: 4
---

# Customize presets

Use an overlay for a small number of customizations. Use an external `presets_dir` when you need to
maintain a complete preset collection.

These modes have different fallback rules:

- with the built-in base, an overlay replaces matching paths and every unmatched path still comes
  from the embedded preset;
- a full external `presets_dir` is authoritative for app and shell categories. Missing content is
  not silently borrowed from the binary.

Shine prints `Preset Source`, optional `Presets Overlay`, and external shell deployment mode so you
can tell which model is active before interpreting `list`, `update`, or install output.

## Create a preset with AI

Shine ships the portable Agent Skill at `skills/shine-preset-author/` in its source and crate
package. Register that directory with your AI client's native skill installer or skills-directory
mechanism, then ask in ordinary language for an app configuration, shell command, system bootstrap,
or customization of a built-in category. Shine does not detect or edit Codex, Claude, Cursor, or
other client configuration.

The skill checks that the installed Shine supports static validation, selects the matching author
reference, scaffolds from the current binary, validates the result as JSON, and performs only an
isolated dry-run. It never links or activates the category and never runs hooks, artifacts,
generators, installation scripts, or a real bootstrap. The skill instructions are English for
cross-client portability, but questions and the final report follow the user's language.

You can use the same flow without an AI client:

```bash
mkdir -p my-presets/app/my-editor
cd my-presets/app/my-editor
shine preset new app
# Add config files and edit shine.toml.
shine preset validate . --format json
```

Use `shell` or `sys` in `preset new` for the other kinds. To customize an embedded category, enter
the repository or overlay root and run `shine preset copy <kind>/<name>`; the command creates the
kind/category path.

`preset validate` also accepts a repository root, one category directory, or its `shine.toml`. Root
validation scans only direct category directories below `app/`, `shell/`, and `sys/`; an empty root
is invalid. It evaluates the macOS, Linux, and Windows declarations on any host, verifies referenced
files and locked Bun dependency policy, and reports compatible metadata-free app/shell categories
with a `legacy_metadata` warning. It does not load active source/overlay settings, initialize config,
check for updates, write files, access the network, or execute preset code.

The default output is text. `--format json` emits the stable `schema_version: 1` report used by the
skill; validation errors exit with status 1, while warnings do not.

## From source folders to installed capabilities

Any tool or process that places a preset folder on a machine can be the synchronization layer.
Shine does not require Git or provide general-purpose folder synchronization. It turns selected
source files into installed capabilities: it creates managed command entries, resolves local values,
keeps an installed snapshot by default, reports pending changes, and removes only what it owns.

The built-in `shell/image-tools/` category is a complete example. It exposes three image commands
through this metadata:

```toml
description = "Personal image workflow commands."

[[files]]
source = "compress.ts"
target = "img-compress"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY"]

[[files]]
source = "resize.ts"
target = "img-resize"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY", "IMAGE_MAX_WIDTH", "IMAGE_MAX_HEIGHT"]

[[files]]
source = "convert.ts"
target = "img-convert"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["IMAGE_QUALITY"]
```

The category includes three entry files plus a shared implementation using
[`Bun.Image`](https://bun.com/docs/runtime/image). It compresses, resizes, and converts JPEG, PNG,
and WebP without ImageMagick, Sharp, or another image library. Every machine that runs the commands
needs Bun 1.3.14 or newer in `PATH`; Shine does not bundle Bun. The commands detect a missing
`Bun.Image` API and print an upgrade hint.

Install the category or one command, then use the same lifecycle as any other shell preset:

```bash
shine info shell/image-tools
shine install shell/image-tools/img-compress
img-compress photo.jpg screenshots/
img-resize --width 1280 --output-dir ./resized photos/
img-convert --format webp --quality 75 --output-dir ./webp hero.png gallery/
shine info shell/image-tools --diff
shine upgrade shell/image-tools
shine shell uninstall image-tools/img-compress --dry-run
```

Each command accepts multiple file or directory inputs. A directory scan processes direct JPEG,
PNG, and WebP children only; it never recurses. Without `--output-dir`, output stays beside its
source as `photo.compressed.jpg`, `photo.resized.jpg`, or the selected conversion extension. An
output directory flattens the selected inputs, so duplicate target names fail explicitly. Existing
files also fail unless `--force` is present, and source images are never modified in place.

Batch processing continues after an individual failure and returns a nonzero status if any item
failed. The first 20 failures appear in the terminal. If there are more, the complete list is also
written to a uniquely named `image-tools-errors-*.log` under `--output-dir`, or the current directory
when no output directory was supplied.

`IMAGE_QUALITY`, `IMAGE_MAX_WIDTH`, and `IMAGE_MAX_HEIGHT` default to `80`, `1920`, and `1080`.
Command options override them for one invocation; changing the Shine values keeps the preference on
that machine. In the default snapshot mode, changing a copied or external source still requires
`shine upgrade` before the installed command changes. This is the boundary between synchronizing a
script file and operating it as a reusable personal capability.

## Override selected files with an overlay

An overlay replaces base-preset files at the same relative path and can add new categories:

```bash
shine preset overlay link ~/dotfiles/shine-overlay --create
shine preset overlay info
shine preset overlay unlink
```

For example, `app/starship/starship.toml` overrides the file at the same path in the base source;
other presets continue to use the base source.

### Mirror an overlay from Git

Shine can manage a local mirror of a read-only overlay repository used by multiple devices:

```bash
shine preset overlay link --git https://example.com/team/shine-overlay.git --branch main
shine preset pull
shine preset overlay info
```

The first `shine preset pull` shallow-clones the repository under `~/.shine/overlay/`. Later pulls
mirror that directory to the latest state of the remote branch. It is a disposable cache: the next
pull discards local edits. Make changes and push them in an upstream checkout, then synchronize each
device with `shine preset pull`, `shine update --pull`, or `shine upgrade --pull`.

To customize one built-in category, copy it at the overlay root instead of exporting everything. For
example, copy Surge before editing its local proxy, group, and rule files:

```bash
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
```

The command accepts complete `app/<name>`, `shell/<name>`, or `sys/<name>` categories. Existing files
are overwritten only with `--force`. The overlay replaces only paths you keep, so delete copied files
you do not customize and let them continue to come from the built-in version. See
[Manage application configuration](./app-presets.md#surge-uri-subscriptions) for the remaining Surge
steps.

## Export the complete preset collection

```bash
shine preset link ~/dotfiles/shine-presets --create
shine preset export
```

After linking an external directory, `install`, `list`, and `update` read from it. Command output
identifies the active preset source.

### Choose how external shell presets are deployed

External shell presets use **snapshot** mode by default. During installation, Shine copies the
effective category under `~/.shine/installed/shell/` and commands execute the managed copy. After
editing the source, run `shine update` to inspect changes and `shine upgrade` to apply them. This gives
shell scripts the same review-before-update flow as application configuration. `update` reports
legacy direct-link installations, and `upgrade` migrates them.

For preset development, explicitly enable **live** mode when linking the source:

```bash
shine preset link ~/dotfiles/shine-presets --live
```

In live mode, ordinary shell and Bun source changes take effect on the next invocation. Files with
`transforms` are rendered atomically before each call; a rendering failure aborts that call rather
than executing stale output. Changes to entry metadata such as `target`, `runtime`, `transforms`, or
`env` still require `shine upgrade` to rebuild the managed entry. Restore snapshot mode by running
`shine preset link <PATH>` without `--live`, or use `shine preset unlink`.

If a linked overlay or live preset directory is moved, link the new path and run `shine update`.
Snapshot deployments stay current when their effective relative files and bytes are unchanged.
Live deployments report the old and new source paths because `shine upgrade` must repoint their
managed command entries; this relocation is shown separately from content changes.

### Use locked packages in external Bun presets

Built-in Bun presets remain self-contained. External presets and overlays may use ordinary registry
packages by committing both `package.json` and `bun.lock` in the same physical category directory as
the effective script:

```text
shell/my-tools/
├── shine.toml
├── package.json
├── bun.lock
├── command.ts
└── shared.ts
```

This convention also applies to Bun app artifact, teardown, and generator scripts under
`app/<category>/`. Both files are required. Shine rejects a lone manifest or lock and, in this first
version, any `trustedDependencies` declaration. An overlay declaration applies only when
the overlay supplies the effective script; adding package files beside an inherited built-in script
does not enable dependencies for it.

Shine runs built-in and unlocked external scripts with `bun --no-install`. A locked external script
runs with `bun --install=fallback`, so its first actual execution may download missing packages.
`list` and `info` never fetch dependencies. Shine does not run `bun install`, copy `node_modules`, or
own Bun's global cache and virtual store; uninstalling Shine or a preset does not clear those shared
caches.

For snapshot Shell presets, package or lock changes appear in `shine update` and take effect after
`shine upgrade`. In live mode they are read on the next command invocation, while status still
reports that the installed receipt should be refreshed. Fully offline machines need the relevant
Bun cache already populated, or a bundled/vendored script. Native extensions, workspaces, `file:`,
`link:`, and dependencies requiring lifecycle scripts are not guaranteed in this version.

To migrate an external script that currently relies on Bun's implicit installation, create its
category-local `package.json`, generate `bun.lock` with the repository's Bun version, commit both,
and test from an empty Bun cache. Without the pair, bare package imports now fail instead of being
downloaded automatically.

You can also select the source through an environment variable:

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine preset export
```

## Create a commit-ready preset repository

```bash
cd ~/dotfiles/shine-presets
shine init
```

This creates `shine.config.toml` and sets `presets_dir` to the current directory. Shine searches from
the working directory upward for the nearest project configuration, so commands work in its
subdirectories. Use `shine init --yes` in non-interactive scripts.

## Pull Git-managed sources

When the external preset directory or a manually linked overlay is a Git worktree, pull sources alone
or pull before inspecting or applying configuration:

```bash
shine preset pull
shine update --pull
shine upgrade --pull
```

Shine locates the repositories containing the base presets and active overlay. If both sources are in
one repository it pulls only once; non-Git sources are skipped. `update --pull` and `upgrade --pull`
reload configuration after pulling, so an updated `shine.config.toml` affects later steps.

Every repository processed must meet these conditions before any pull begins:

- the worktree has no tracked or untracked changes;
- `HEAD` is on a branch, not detached;
- the current branch has an upstream.

These sources use `git pull --ff-only`. Shine never stashes, rebases, resets, or resolves conflicts.
Validation stops before modifying any repository. These restrictions do not apply to an overlay
managed with `--git`, which is intentionally disposable. Git must be available in `PATH`.

## Create category metadata

Generate a `shine.toml` template in an application, shell, or sys category directory:

```bash
shine preset new app
shine preset new shell
shine preset new sys
```

Existing files require `--force`. Category metadata is a preset-author interface; after editing it,
first run `shine preset validate . --format json`, then use the relevant isolated installation
`--dry-run` command. Shell install dry-run resolves intended command links without creating files,
links, manifests, snapshots, rendered files, or profile edits.

### Give an application file its own destination

An application category has a default `dest`, but an explicit `[[files]]` entry may override that
root. `target` stays relative to the selected root:

```toml
dest = "~/.config/my-app"

[[files]]
source = "config.toml"
target = "config.toml"

[[files]]
source = "shared/rules.list"
target = "rules/provider.list"
dest = { base = "data-dir", path = "com.example.my-app" }
```

The override accepts the same absolute string or platform mapping as category destinations.
Mappings accept exact `macos`, `linux`, and `windows` keys plus `unix` as a macOS/Linux fallback;
an exact key wins when both are present. A missing branch omits that category or file on the
corresponding OS. `platforms` arrays on explicit App and Shell files use the same four selectors,
combine them with OR semantics, and must not be empty. The structured `data-dir` form is file-only
and resolves the platform's user
application-data root: `%APPDATA%` on Windows, Application Support on macOS, and `XDG_DATA_HOME`
(or `~/.local/share`) on Linux. `path` and `target` must be relative and cannot contain `..`.

Shine rejects two entries that resolve to the same destination before writing anything. If a later
metadata revision moves an already managed source, `shine upgrade` moves it only when the old copy
is unmodified and the new destination is free. Otherwise both locations are left untouched for the
user to resolve.

## Shell entries with an optional runtime

`runtime` selects the runtime for a shell preset command entry; it does not add an interactive shell.
Without it, entries use native `.sh` or `.ps1` files. The only configurable alternative today is
`bun`:

```toml
[[files]]
source = "my-tool.ts"
target = "my-tool"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["API_URL", "SERVICE_TOKEN=API_TOKEN"]
```

Supported extensions are `.ts`, `.js`, `.mts`, and `.mjs`. Shine creates a managed entry without an
extension, which users invoke as `my-tool`. Existing native `.sh` and `.ps1` entries remain
compatible.

Every device that runs the command needs Bun in `PATH`. Shine does not install Bun or manage
`node_modules`; external categories may opt into Bun-managed locked packages as described above.
`runtime = "bun"` cannot be combined with `needs_source = true`.

The optional `env` list applies only to Bun entries. Each item is `KEY` or `SOURCE=TARGET`. The entry
injects values through `shine env run --no-workspace --with ...`, preferring `SOURCE_SECRET` and then
plain `SOURCE`. When `env` is declared, the machine must also have `shine` in `PATH`. Declare only key
names in metadata—never values or ciphertext.

Future runtimes will be documented here with their values, file types, prerequisites, and limits.
Python, Node, and Deno are not currently valid `runtime` values.

## Author a system bootstrap item

A sys category is one OS directory such as `sys/ubuntu/`. Every executable sys preset declares
`version = 2`, then describes ordinary ensure-present software with detection and a fixed provider:

```toml
version = 2

[[items]]
id = "mise"
label = "mise"
description = "Install mise without managing its versions."

[items.detect]
kind = "command"
command = "mise"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew" # homebrew-cask, apt, or winget are also supported
package = "mise"

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]

[profiles.recommended]
items = ["mise"]
```

Detection supports `command`, `path`, and `any` command/path probes. Package installs are fixed
ensure-present actions: Shine owns argv, elevation, proxy handling, timeout, output limits, and the
post-install detection, but never upgrades the package. A complex item may use
`[items.install] kind = "script", path = "install/<item>.sh"`; the script handles only that item,
returns a normal exit code. Every init item must declare both `detect` and `install`; there is no
platform-wide dispatcher fallback. Version 1 manifests are rejected before detection or profile
writes; see [the v2 migration guide](sys-preset-v2-migration.md).

Shell integrations accept exactly one of `path`, `env`, `eval`, `source`, `aliases`, or `fragment`.
Use `profile/base.pre.sh` and `profile/base.post.sh` only for OS-wide content; put complex item logic
in `profile/<item>.sh`. Phase, optional `priority`, manifest order, and declaration order determine
stable composition. Named `[profiles.*]` tables select bootstrap items; they do not define shell
content or disable integrations outside the selection.

External sys install scripts and executable profile content (`eval`, `source`, fragments, and base
files) require the user to review the source and set `allow_sys_code = true` in the global config;
the project config cannot authorize itself. If executable sys code is blocked during bootstrap
preflight, the error identifies the code kind and path when available, each active external preset
layer, and the global config path. It presents separate actions to grant permission or keep external
code blocked; no installer has run yet. Static detection, package metadata, PATH, env, and aliases
remain available without that opt-in. Validate with
`shine sys list`, `shine sys info <ITEM>`, and `shine sys bootstrap <ITEM> --dry-run`.

## Application artifact runtimes

An application's `[artifact]` can also use Bun for cross-platform apply and teardown scripts:

```toml
[artifact]
script = "build.ts"
teardown = "unbuild.ts"
runtime = "bun"
```

The default is `native`, which executes the script directly. `bun` accepts only `.ts`, `.js`, `.mts`,
or `.mjs` and requires Bun on the machine. Artifact scripts receive the current Shine `[env]` and
application path variables. To run an artifact automatically after installation or upgrade actually
changes files, separately declare `post_install` or `post_upgrade`; external presets still require
the user to set `allow_app_hooks = true`.
