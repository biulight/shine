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

Generate a `shine.toml` template in an application or shell category directory:

```bash
shine preset new app
shine preset new shell
```

Existing files require `--force`. Category metadata is a preset-author interface; after editing it,
validate with the relevant `list`, `info`, and installation `--dry-run` commands.

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

The override accepts the same absolute string or `{ windows = "...", unix = "..." }` mapping as
category destinations. The structured `data-dir` form is file-only and resolves the platform's user
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

Every device that runs the command needs Bun in `PATH`. Shine does not install Bun, download
dependencies, or resolve `node_modules`. `runtime = "bun"` cannot be combined with
`needs_source = true`.

The optional `env` list applies only to Bun entries. Each item is `KEY` or `SOURCE=TARGET`. The entry
injects values through `shine env run --no-workspace --with ...`, preferring `SOURCE_SECRET` and then
plain `SOURCE`. When `env` is declared, the machine must also have `shine` in `PATH`. Declare only key
names in metadata—never values or ciphertext.

Future runtimes will be documented here with their values, file types, prerequisites, and limits.
Python, Node, and Deno are not currently valid `runtime` values.

## Author a system bootstrap item

A sys category is one OS directory such as `sys/ubuntu/`. Set `profile_composition = true` in its
`shine.toml`, then describe ordinary ensure-present software with detection and a fixed provider:

```toml
profile_composition = true

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
returns a normal exit code, and must not emit the legacy `SHINE_SYS_STATUS` protocol.

Shell integrations accept exactly one of `path`, `env`, `eval`, `source`, `aliases`, or `fragment`.
Use `profile/base.pre.sh` and `profile/base.post.sh` only for OS-wide content; put complex item logic
in `profile/<item>.sh`. Phase, optional `priority`, manifest order, and declaration order determine
stable composition. Named `[profiles.*]` tables select bootstrap items; they do not define shell
content or disable integrations outside the selection.

External sys install scripts and executable profile content (`eval`, `source`, fragments, and base
files) require the user to review the source and set `allow_sys_code = true` in the global config;
the project config cannot authorize itself. Static detection,
package metadata, PATH, env, and aliases remain available without that opt-in. Validate with
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
