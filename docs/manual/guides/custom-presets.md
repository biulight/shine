---
title: Customize presets
sidebar_position: 4
---

# Customize presets

Use an overlay for a small number of customizations. Use an external `presets_dir` when you need to
maintain a complete preset collection.

Choose a starting point:

| Goal | Start here |
| --- | --- |
| Customize a few built-in files | [Link an overlay](#override-selected-files-with-an-overlay) |
| Maintain your own complete collection | [Export presets](#export-the-complete-preset-collection) |
| Create a new App, Shell, or Sys category | [Create category metadata](#create-category-metadata) |
| Use an AI assistant to draft a preset | [Create with AI](#create-a-preset-with-ai) |
| Try a category you have written | [Preview and install](#preview-and-install-your-category) |

The source choices have different fallback rules:

- with the built-in base, an overlay replaces matching paths and every unmatched path still comes
  from the built-in preset;
- a full external `presets_dir` is authoritative for app and shell categories. Missing content is
  not filled in from built-in presets.

Check `Preset Source` and `Presets Overlay` in command output to confirm which files Shine is using.
Replace example paths and `<PATH>` with your own directory; quote paths containing spaces.

## Override selected files with an overlay

An overlay replaces base-preset files at the same relative path and can add new categories:

```bash
shine preset overlay link ~/dotfiles/shine-overlay --create
shine preset overlay info
```

When you want to stop using this overlay:

```bash
shine preset overlay unlink
```

Unlink removes overlay settings from the active configuration; it does not delete source files.
Removing a project override can restore an inherited global overlay. Run `shine preset overlay info`
to inspect the effective overlay afterward.

For example, `app/starship/starship.toml` overrides the file at the same path in the base source;
other presets continue to use the base source.

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

### Mirror an overlay from Git

Shine can manage a local mirror of a read-only overlay repository used by multiple devices:

Git-managed overlays are global settings. Run the following commands outside any Shine project:
neither the current directory nor its ancestors may contain a `shine.config.toml`.
For a project-specific overlay, clone the repository yourself
and use `shine preset overlay link <PATH>` from the project instead.

```bash
shine preset overlay link --git https://example.com/team/shine-overlay.git --branch main
shine preset pull
shine preset overlay info
```

Treat the managed mirror as read-only: pulling replaces its contents, including local edits and
extra files. Edit and push changes from a separate checkout, then run `shine preset pull` on each
device. Use a manually linked overlay if you need to keep local edits. Without `--branch`, Shine
follows the remote's default branch. If Shine reports an unexpected directory at the mirror path,
move it aside before retrying.

## Export the complete preset collection

```bash
shine preset link ~/dotfiles/shine-presets --create
shine preset export
```

Export includes built-in presets plus overrides and additions from the active overlay. It does not
copy files that exist only in the external base directory. `preset copy` always extracts the original
built-in category.

Both `preset copy` and `preset export` reject linked or special output files and linked
subdirectories, including with `--force`. Choose an ordinary directory tree for the output.
Without `--force`, existing regular files are preserved.

After linking an external directory, `install`, `list`, and `update` read from it. Command output
identifies the active preset source.

You can also select the source for one command through an environment variable:

```bash
SHINE_PRESETS=~/dotfiles/shine-presets shine preset export
```

To stop using the external collection, run `shine preset unlink`. This removes the source setting,
not its files or installed presets. Project inheritance and `SHINE_PRESETS` may still select another
source; check `Preset Source` afterward.

### Choose how external shell presets are deployed

External Shell presets use **snapshot** mode by default: installed commands keep using the
reviewed copy until you update them. After editing the source, run `shine update` to inspect changes
and `shine upgrade` to apply them.

For preset development, explicitly enable **live** mode when linking the source:

```bash
shine preset link ~/dotfiles/shine-presets --live
```

Once installed in live mode, Shell and Bun source edits take effect on the next invocation.
Template substitutions also refresh on each invocation; a failed substitution stops the command.
Changes to `target`, `runtime`, `transforms`, or `env` still require `shine upgrade`.

To return to snapshot mode while keeping the same source, run
`shine preset link ~/dotfiles/shine-presets` without `--live`. After switching either way, run
`shine update shell/<CATEGORY>` and `shine upgrade shell/<CATEGORY>` for each installed category
you want to switch. Linking alone does not change already installed commands.

Live mode requires [development trust](#declare-permissions), which also authorizes future code
changes. If you move a linked source, link the new path and run `shine update`; apply any reported
changes with `shine upgrade`.

### Use locked packages in external Bun presets

Built-in Bun presets remain self-contained. External presets and overlays may use ordinary registry
packages by committing both `package.json` and `bun.lock` in the same category directory as
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
`app/<category>/`. Both files are required; `trustedDependencies` is not allowed.
An overlay declaration applies only when
the overlay supplies the effective script; adding package files beside an inherited built-in script
does not enable dependencies for it.

Scripts without the required manifest and lock file cannot download packages automatically.
A locked external script may download missing packages on its first execution; `list` and `info`
do not fetch them. Include the manifest and lock file when sharing a preset; do not rely on a local
`node_modules` directory being carried to another machine.

For snapshot Shell presets, package or lock changes appear in `shine update` and take effect after
`shine upgrade`. In live mode they are read on the next command invocation, while status still
reports that the installed command should be refreshed. Fully offline machines need the relevant
Bun cache already populated, or a bundled/vendored script. Native extensions, workspaces, `file:`,
`link:`, and dependencies requiring lifecycle scripts are not guaranteed currently.

Generate and commit `bun.lock` with the Bun version your preset supports. Before sharing the
preset, test dependency loading with an empty Bun cache.

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

Choose one pull/update operation:

| Operation | Command |
| --- | --- |
| Pull preset sources only | `shine preset pull` |
| Pull, then check for updates | `shine update --pull` |
| Pull, then apply configuration updates | `shine upgrade --pull` |

Git must be available in `PATH`. Commit or set aside your edits before pulling. Changes pulled
into `shine.config.toml` also affect the following `update` or `upgrade`.

Every repository processed must meet these conditions before any pull begins:

- the worktree has no tracked or untracked changes;
- `HEAD` is on a branch, not detached;
- the current branch has an upstream.

These sources use `git pull --ff-only`. Shine never stashes, rebases, resets, or resolves conflicts.
Resolve reported worktree or branch problems yourself before retrying. These restrictions do not
apply to an overlay managed with `--git`, where pulling replaces the local mirror.

## Create category metadata

Each category has its own directory and `shine.toml`. For example, create an App category:

```bash
mkdir -p ~/dotfiles/shine-overlay/app/my-editor
cd ~/dotfiles/shine-overlay/app/my-editor
shine preset new app
```

This creates metadata only. Add `config.toml` beside it and set `dest` in `shine.toml` to the
application's configuration directory. Keep the generated `metadata_schema_version = 2`.

For other category types, create a directory under `shell/<name>` or `sys/<os>` instead and run
the corresponding command there:

| Operation | Command |
| --- | --- |
| Create an App template | `shine preset new app` |
| Create a Shell template | `shine preset new shell` |
| Create a Sys template | `shine preset new sys` |

To start from a built-in category, run `shine preset copy <kind>/<name>` from your repository or
overlay root. It creates the kind/category path.

Existing files require `--force`; review them before replacing the template. Add the configuration
files or scripts referenced by `shine.toml` and declare script access and required inputs as described
[below](#declare-permissions). Then [validate the category](#validate-and-package-a-preset) and
[preview its installation](#preview-and-install-your-category).

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

Legacy App presets without `shine.toml` still support a `# shine-dest:` full destination path
on the first line, or immediately after a shebang. The path must be absolute after home expansion
and cannot contain `..`. Replace parent components with an explicit destination before retrying.

Shine rejects two entries that resolve to the same destination before writing anything. If a later
metadata revision moves an already managed source, `shine upgrade` moves it only when the old copy
is unmodified and the new destination is free. Otherwise both locations are left untouched for the
user to resolve.

## Create a preset with AI

Shine provides an Agent Skill in the source repository at `skills/shine-preset-author/`.
Install it through your AI client's skill installer or skills directory, then describe the App
configuration, Shell command, system setup, or built-in customization you want.

The skill generates a preset, validates it, and previews installation in an isolated directory.
It does not activate the preset or run its scripts. Review the result before installing it.

## Study a built-in example {#from-source-folders-to-installed-capabilities}

For a complete Shell example, copy the built-in image tools into your overlay:

```bash
cd ~/dotfiles/shine-overlay
shine preset copy shell/image-tools
```

Read `shell/image-tools/shine.toml` alongside its scripts to see how command names, Bun, supported
platforms, and environment inputs fit together. Command usage is documented in the
[image workflow](../reference/built-in-presets.md#image-workflow).

## Declare runtime environment inputs

Shell Bun commands and App generators, hooks, and artifacts use structured `env` entries:

```toml
env = [
  { source = "API_URL", sensitivity = "plain" },
  { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" },
]
```

`source` names the value in Shine configuration; `target` is the name the script reads and defaults
to `source`. Declare only names and sensitivity, never values or ciphertext. Set values separately
using [environment configuration](./environment.md). Each entry receives only its declared Shine
inputs. Keep sensitivity consistent for the same source; `sensitivity = "secret"` does not itself
encrypt a value. A generator's `when_env` must name a source in its `env` list.

Old string arrays such as `env = ["SOURCE=TARGET"]` are no longer accepted. Use
[metadata migration](#migrate-a-1x-source), then review any missing or conflicting sensitivity
declarations. The command-line `shine env run --with SOURCE=TARGET` syntax is unchanged.

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
env = [{ source = "API_URL", sensitivity = "plain" }, { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" }]
```

Supported extensions are `.ts`, `.js`, `.mts`, and `.mjs`. After installation, invoke the command
as `my-tool`, without the source-file extension.

Every device that runs the command needs Bun in `PATH`. Shine does not install Bun or manage
`node_modules`; external categories may opt into Bun-managed locked packages as described above.
`runtime = "bun"` cannot be combined with `needs_source = true`.

The optional `env` list applies only to Bun entries and follows the [environment input format](#declare-runtime-environment-inputs).
For each declared source, Shine prefers `SOURCE_SECRET`, then plain `SOURCE`. It does not add the
workspace's default secret bindings. When `env` is declared, the machine must also have `shine` in
`PATH`. Declare only names and sensitivity in metadata, never values or ciphertext.

Python, Node, and Deno are not currently valid `runtime` values.

## Author a system bootstrap item

A Sys category is one OS directory such as `sys/macos/`. Declare `version = 2`, then describe how
to detect and install each program. This example uses Homebrew on macOS:

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
provider = "homebrew" # homebrew-cask, apt, pacman, or winget are also supported
package = "mise"

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]

[profiles.recommended]
items = ["mise"]
```

Use `provider = "pacman"` only in `sys/arch/`, with one package name per item. It uses the
existing pacman databases; complete a full system update before bootstrap as described in
[Arch Linux preparation](system-init.md#arch-linux-preparation).

Each item needs both `detect` and `install`. Detection can check a `command`, a `path`, or `any`
of several command/path checks. Package installation ensures the program is present without
upgrading an existing installation. For custom installation logic, use `kind = "script"` and
`path = "install/<item>.sh"` under `[items.install]`; that script should handle only its own item.
For version 1 presets, follow [the v2 migration guide](sys-preset-v2-migration.md).

Shell integrations accept exactly one of `path`, `env`, `eval`, `source`, `aliases`, or `fragment`.
Use `phase = "pre"` or `"post"` to choose when an integration loads. Within a phase, lower
`priority` values load first; ties follow the order in `shine.toml`. Use `profile/base.pre.sh` and
`profile/base.post.sh` only for OS-wide content; put complex item logic in `profile/<item>.sh`.
Named `[profiles.*]` tables select bootstrap items; they do not define shell
content or disable integrations outside the selection.

External Sys installation scripts and executable shell configuration need
[code authorization](#declare-permissions). After linking your source, inspect it with `shine sys list`
and `shine sys info <ITEM>`, then preview with `shine sys bootstrap <ITEM> --dry-run` before applying.

## Application artifact runtimes

An application's `[artifact]` can also use Bun for cross-platform apply and teardown scripts:

```toml
[artifact]
script = "build.ts"
teardown = "unbuild.ts"
runtime = "bun"
env = [{ source = "PROFILE_PATH", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }]
```

The default is `native`, which executes the script directly. `bun` accepts only `.ts`, `.js`, `.mts`,
or `.mjs` and requires Bun on the machine. Only Shine `[env]` variables listed in `env` are passed
to the artifact. Each entry declares its source, optional target alias, and sensitivity together; no duplicate
`[permissions].environment` entry is needed. Listed sources are forwarded only when configured; missing
optional values are omitted. Fixed application path variables are listed below. External presets
require human confirmation of the operation plan or a matching trust grant.

To run the artifact, use `shine app artifact apply <CATEGORY>` and review the plan. See
[application artifacts](./tasks-and-serve.md#generate-application-helper-resources) for the workflow.

### Run a script after an App configuration changes

Declare `post_install` or `post_upgrade` at the top level of `shine.toml`, before any table headers.
Each accepts one hook or a list. For example:

```toml
post_upgrade = [{ script = "refresh.ts", runtime = "bun", env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }] }]

[permissions]
schema_version = 1
network = [{ scope = "host", host = "api.example.com" }]
```

Use exactly one of `command` or `script` per hook. A command hook runs a program with optional
`args`; a script hook runs a category-relative script and may specify `runtime = "bun"`.
`runtime` is valid only with `script`. Use the same Bun file extensions and dependency rules as
artifacts. Declare the actual access needed by your script; the example assumes requests to
`api.example.com`.

Command hooks require their declared inputs to be configured. Script hooks omit absent optional
inputs, so the script must handle them. Required missing inputs block approval. External hooks
need the same review or trust as other App code. Do not call artifact lifecycle commands from a
hook; use an explicit artifact operation instead. See [hook behavior](./app-presets.md#lifecycle-hooks)
for when hooks run and how failures are reported.

### Paths available to App scripts

Generators, script hooks, artifacts, and teardown scripts receive these variables without an `env`
declaration. Command hooks do not receive this App-specific set.

| Variable | Use |
| --- | --- |
| `SHINE_APP_ID` | The application category name |
| `SHINE_APP_DIR`, `SHINE_APP_SOURCE_DIR` | The files provided to the current script; also its working directory |
| `SHINE_APP_OVERLAY_DIR` | Overlay files for the current run; empty when there is no overlay |
| `SHINE_CONFIG_DIR` | The active Shine configuration and state root |
| `SHINE_STATE_DIR` | Persistent output for this application category |
| `SHINE_CACHE_DIR` | Cache output for this application category |

The script source directories are temporary. Do not save their paths or write persistent results
there; use `SHINE_STATE_DIR` or `SHINE_CACHE_DIR`, creating the output directory when needed.

## Describe capabilities and required inputs {#declare-permissions}

Declare the access your scripts need in `shine.toml`. Use `[permissions]` for an App category,
`[files.permissions]` for a Shell command, or `[items.permissions]` for a Sys item. Tables are
optional, but App code checks require declarations for the behavior they identify.

For example:

```toml
[permissions]
schema_version = 1
filesystem = [
  { access = ["read", "write"], base = "home", path = ".config/example" },
]
network = [{ scope = "host", host = "api.example.com" }]
```

For Shell and Sys categories, use `permission_defaults` to apply one declaration to entries that
do not provide their own target-local override:

```toml
[permission_defaults]
schema_version = 1
network = [{ scope = "host", host = "api.example.com" }]

[[files]]
source = "foo.sh"
target = "foo"

[[files]]
source = "bar.sh"
target = "bar"
```

An entry's own permission table replaces `permission_defaults` for that entry.

Filesystem bases are `home`, `shine`, `data-dir`, `preset`, or `absolute`. Except with `absolute`,
paths must be relative and contain no `..`; `.` means the base directory itself. Commands contain a program
name without arguments. Environment declarations contain names and `plain`/`secret` sensitivity,
never values. Do not repeat inputs, commands, script paths, or destinations already declared in
other metadata fields.

**Scripts are not sandboxed.** They run with their process's access. Permission statements,
including schema-v2 `opaque_code = "unrestricted"`, do not grant trust or administrator access.
Declare environment inputs explicitly, and add `administrator = true` when administrator access
is required.

### Authorize external code

For an interactive installation or update, review the operation plan and confirm the listed code.
This authorizes only that operation and the reviewed content, including Shell commands installed
for later use. Changes after review require a new approval. `--yes` cannot provide this consent;
automation needs a matching trust grant already in place.

For persistent authorization, inspect the target, then choose the scope you need:

| Operation | Command |
| --- | --- |
| Inspect code and required access | `shine trust inspect <TARGET>` |
| Trust the reviewed version | `shine trust grant <TARGET>` |
| Also trust future code changes | `shine trust grant <TARGET> --development` |
| List grants | `shine trust list` |
| Revoke a grant | `shine trust revoke <TARGET>` |

Targets are `app/<CATEGORY>`, `shell/<CATEGORY>/<COMMAND>`, or `sys/<ITEM>`. `preset` selects all executable
targets in the active presets; `shine trust revoke preset` clears all stored preset grants.

Snapshot trust requires renewed review when category files, metadata, source layers, or executable
permissions change. It does not verify external dependencies, interpreters, tools, or downloads.
Development trust also accepts future code changes from the selected local source. A source move,
new target, or new capability requires another grant. Use it only if you accept that scope;
Shell live mode requires it. Choose snapshot mode if you want to review each update.

Explicit `info`/`update --run-generators` also requires persistent trust. For one-operation
generator approval, use interactive `shine app refresh`. If an old grant is marked as needing
review, inspect the target and grant it again.

AI-generated code needs your review too. Development trust on an AI-writable directory also
authorizes future AI edits for the covered targets.

## Validate and package a preset

Run these checks from the category directory:

```bash
shine preset validate .
shine preset lint .
shine preset plan . --platform macos
```

`validate` checks metadata and referenced files. It also accepts a preset repository root or a
`shine.toml` file. Both checks cover supported platform declarations regardless of your current OS.
`lint` reports portability and code-declaration issues; fix errors and review warnings before
proceeding. If lint cannot load the category, use `validate` to find the input error.

Run `plan` for each platform you support: `macos`, `linux`, or `windows`. It shows a hypothetical
first installation, including required permissions and anything that would block it. The preview
uses simulated conditions, so missing inputs or trust in that report do not describe your current
machine. Use an installation `--dry-run` to inspect your actual setup. These checks do not run preset scripts
or change your installed configuration; a successful preview is not installation approval.

For repeatable tests, add `shine.test.toml` to the category and run `shine preset test .`. Tests
describe starting conditions and expected results without executing scripts. The source repository's
`examples/presets/` directory contains App, Shell, and Sys examples. Use relative paths and supported
placeholders such as `${HOME}` in tests that must work on other machines. `plan` and `test` accept
one category at a time.

To distribute a reviewed category, write the bundle outside its source directory:

```bash
shine preset pack . --output ../../my-editor.shine-preset.tar.gz
```

The bundle excludes `shine.test.toml`. Use `--force` only to replace an existing output; it does not
bypass validation or allow output inside the category, including through a symbolic link.

## Check App code before execution

Before distributing App scripts, run:

```bash
shine preset lint <PATH>
```

Shine checks Bun generators, script hooks, artifacts, teardown scripts, and supported local
JavaScript/TypeScript dependencies. Fix blocking findings at the reported file and line: declare
the access the script needs, or change the script to match its declarations. Parse failures also
block execution. Trust, `--yes`, and `opaque_code = "unrestricted"` do not bypass these checks.

Common declarations include:

| Script behavior | Required declaration |
| --- | --- |
| Fetch from `https://example.org/data` | Network host `example.org` |
| Read `in` with `Bun.file('in').text()` | Filesystem `read` on `in` |
| Copy a file with Node `copyFile` | `read` on the source and `write` on the destination |
| Rename a file with Node `rename` | `remove` on the source and `write` on the destination |
| Read a configured environment value | An `env` entry with `source`, optional `target`, and `sensitivity` |

The script reads the `target` name from an environment mapping. Only documented fixed variables
are exempt from declaration; a `SHINE_` prefix alone is not an exemption. Never put environment
values or URL credentials in capability declarations.

Run `lint` even if `plan` or `test` succeeds: those commands check only scripts selected by their
simulated inputs, while lint checks all App script entries, including inactive generators.

Static checks cannot cover every effect of a script. Passing lint does not prove that code is
safe; review the code and complete the normal authorization steps before running it.

## Preview and install your category

After validation, link the directory containing `app/`, `shell/`, or `sys/`, not the category
directory itself. For the `my-editor` example above:

```bash
shine preset overlay link ~/dotfiles/shine-overlay
shine app info my-editor
shine app install my-editor --dry-run
```

Check the source, destination, required inputs, and permissions in the output. When ready, install
and confirm the plan:

```bash
shine app install my-editor
```

After editing this installed preset, inspect changes with `shine update app/my-editor` and apply
them with `shine upgrade app/my-editor`. Unlinking the source does not uninstall it; use
`shine app uninstall my-editor --dry-run` to preview removal, then omit `--dry-run` to proceed.

For Shell categories, use `shine shell install <CATEGORY> --dry-run`, then install without
`--dry-run` and open a new terminal. For Sys items, use `shine sys bootstrap <ITEM> --dry-run`
before applying. See the [Shell guide](./shell-presets.md), [App guide](./app-presets.md), and
[system guide](./system-init.md) for prerequisites, updates, removal, and recovery limits.

## Tool compatibility {#tool-compatibility}

For editors and CI, add `--format json` to the authoring commands. Validation errors fail the
command; warnings do not. Use `preset lint --deny-warnings` if reviewed warnings should also fail CI.

Run `shine preset schema --format json` for the report and `shine.test.toml` formats supported by
your installed version. After upgrading Shine, check that your tools still support these formats.
Treat `complete = false` or a nonempty `unfinished_cases` list as an incomplete check and resolve
the reported issue before accepting the result.

## Migrate a 1.x source

Preview the active external source and overlay, or name a repository, category, or manifest:

| Operation | Command |
| --- | --- |
| Preview active external sources | `shine preset migrate --dry-run` |
| Preview this source's migration | `shine preset migrate ./my-presets --dry-run` |
| Apply after reviewing the preview | `shine preset migrate ./my-presets` |

Review the metadata diff before confirming; the default answer is No. Migration changes only
`shine.toml` and backs it up before writing. It does not change scripts, data files, environment
values, installed resources, or trust. Resolve entries reported as needing manual migration.
Version 1 Sys presets require the [v2 migration guide](./sys-preset-v2-migration.md).

Use `--yes` only after review. For automated inspection, use `--format json --dry-run`; applying
with JSON output also requires `--yes`. JSON reports do not include file contents or diffs.

A Git-managed overlay is read-only. Migrate and validate its upstream checkout, commit and push
the changes, then run `shine preset pull` on each consuming device.

If migration reports mixed LF/CRLF line endings, choose one style for that file and retry.

After migration, validate the category and use the normal install or upgrade command to apply it.
Review changed external code again when prompted.
