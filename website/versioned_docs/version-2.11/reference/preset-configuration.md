---
title: Preset configuration (shine.toml)
sidebar_position: 3
---

# Preset configuration (shine.toml)

Use this reference when writing a category's `shine.toml`. For creating, linking, authorizing, and
validating a category, follow [Customize presets](../guides/custom-presets.md). Global and project
settings belong in [Configuration reference](./configuration.md), not in this file.

| Location | Configuration |
| --- | --- |
| `app/<category>/shine.toml` | Application files, generators, hooks, and artifacts |
| `shell/<category>/shine.toml` | Installed shell commands |
| `sys/<os>/shine.toml` | Software installation items, shell integrations, and managed system settings |

Put category-level fields before `[artifact]`, `[[files]]`, or other TOML table headers.

## Shared rules {#shared-rules}

### Platforms and paths {#platforms-and-paths}

App and Shell `files.platforms` accept a nonempty array of `macos`, `linux`, `windows`, or `unix`.
Selectors are combined as alternatives; `unix` covers macOS and Linux. Omission adds no platform
filter, but the destination or native script type can still exclude a platform.

App `dest` accepts an absolute path (including a home-relative path such as `~/.config/example`)
or a platform map. Exact platform keys take precedence over `unix`; a missing branch excludes
that category or file. A file override cannot enable a platform excluded by the category destination. Source paths are relative to the category; App `target` paths are relative
to the selected destination. Do not use `..` in these paths.

File-level App `dest` also accepts `{ base = "data-dir", path = "com.example.app" }`.
The base is `%APPDATA%` on Windows, `~/Library/Application Support` on macOS, and `XDG_DATA_HOME`
(or `~/.local/share`) on Linux. `path` and `target` must be relative. Category-level `dest` does
not accept this structured form.

### Script runtimes {#runtimes}

| Value | Meaning and requirements |
| --- | --- |
| `native` (default) | Run the native entry. Shell commands use `.sh` for Unix shells or `.ps1` for PowerShell; App scripts execute directly and need a suitable executable entry. |
| `bun` | Run with Bun installed in `PATH`. Entry scripts support `.ts`, `.js`, `.mts`, and `.mjs`. |

Applies to Shell `files.runtime` and App script hooks, generators, and artifacts. Sys installers
have no `runtime` field. Bun Shell entries cannot use `needs_source = true`.

For Bun installation and language behavior, use the [Bun documentation](https://bun.sh/docs).
For registry packages in external presets, follow [locked dependencies](../guides/custom-presets.md#use-locked-packages-in-external-bun-presets).

### Transforms {#transforms}

`transforms` is an ordered string array, empty by default. Supported values are:

| Value | Effect |
| --- | --- |
| `template` | Replace `@@VARIABLE@@` using Shine environment values; missing variables fail the operation. It does not quote or escape values for the target language. |
| `jsonc-to-json` | Convert JSONC input to JSON. |

For example, `transforms = ["template", "jsonc-to-json"]` substitutes before converting.
App files also accept the single-string `transform` shorthand; do not combine it with `transforms`.

### Environment inputs {#environment-inputs}

Shell Bun entries and App hooks, generators, and artifacts accept an `env` array. It defaults to
empty except for generators, which must declare at least their trigger input:

```toml
env = [
  { source = "API_URL", sensitivity = "plain" },
  { source = "SERVICE_TOKEN", target = "API_TOKEN", sensitivity = "secret" },
]
```

| Field | Default | Meaning |
| --- | --- | --- |
| `source` | Required string | Variable name in Shine configuration. |
| `target` | Same as `source` | Variable name received by the script or command; targets must be unique within the list. |
| `sensitivity` | Required string | `plain` or `secret`; declares sensitivity for review, without encrypting a value. |

Use variable names, not values or ciphertext. Names start with an ASCII letter or `_`, followed by ASCII letters,
digits, or `_`. Declarations for the same source must agree on sensitivity. These inputs already
supply their environment permission declaration; do not duplicate them in `permissions.environment`.
Old string arrays such as `env = ["SOURCE=TARGET"]` require [migration](../guides/custom-presets.md#migrate-a-1x-source).

Shell input injection requires `shine` in `PATH`, prefers `SOURCE_SECRET` over plaintext `SOURCE`,
and does not add workspace default secret bindings. App inputs read the exact `source` name from
Shine configuration; they do not automatically look up `SOURCE_SECRET` or decrypt its value.
Artifacts and script hooks omit unconfigured optional inputs; generators and command hooks require
their listed inputs. Declaring `env` does not isolate a process from its inherited environment. Configure and encrypt actual values through the [environment guide](../guides/environment.md).

## App categories {#app}

### Category fields {#app-category}

| Field | Default | Meaning |
| --- | --- | --- |
| `metadata_schema_version` | `1` (legacy) | Supported versions: `1`, `2`, `3`. Use `2` for new ordinary presets, or `3` when declaring `secret_files`. |
| `description` | None | Human-readable category description. |
| `dest` | Required | Default destination root; see [paths](#platforms-and-paths). |
| `list_mode` | `files` with explicit `files`, otherwise `category` | Choose `files` or `category` for list presentation. |
| `files` | Discover category files | Explicit file list. Without it, files other than `shine.toml` are discovered; a nonempty `secret_files` list disables this implicit discovery. Use explicit entries to keep helper scripts out of installation targets. |
| `secret_files` | `[]` | Workspace-backed encrypted file groups; see [secret files](#secret-files). |
| `artifact` | None | Scripts for explicit apply/remove operations. |
| `post_install`, `post_upgrade` | None | One hook or a nonempty array of hooks. See [hooks](#hooks). |
| `permissions` | None | Category-wide [capability declaration](#permissions). |

### File entries: `[[files]]` {#app-files}

| Field | Default | Meaning |
| --- | --- | --- |
| `source` | Required string | Existing category-relative input file, also required when a generator is declared. |
| `target` | Same as `source` | Relative installed path under the effective `dest`. |
| `dest` | Category `dest` | Override the destination for this file; also accepts the structured data-directory form. |
| `description`, `display_name` | None | Description and optional display label; neither changes the installed path. |
| `platforms` | No additional filter | Supported [platform selectors](#platforms-and-paths). |
| `transforms` / `transform` | No transforms | Apply the [transform pipeline](#transforms); the two forms are mutually exclusive. |
| `install_mode` | `copy` | `copy` replaces the file; `json-merge` updates selected top-level JSON keys. Both retain normal ownership/conflict checks. |
| `managed_keys` | Required for `json-merge` | Nonempty array of unique, nonempty top-level key names without dots. Invalid with `copy`. |
| `requires_admin` | `false` | Declare that installing this file requires administrator access; does not grant access. |
| `restart_hint` | None | Text telling the user how to activate changes; does not restart the application. |
| `generator` | None | Generate this file's content; see below. |

JSON merge requires an object after transforms, containing every managed key. Each managed value
is replaced as a whole; nested settings inside it are not independently preserved. Keys outside
`managed_keys` are preserved. Duplicate final destinations are rejected. For updates, backup,
uninstall, and recovery behavior, see [Manage application configuration](../guides/app-presets.md).

For legacy App presets without `shine.toml`, `# shine-dest:` on the first line or immediately
after a shebang specifies the full destination filename. It must be absolute after home expansion
and contain no `..`. New presets should declare `dest` and `target` in metadata.

### File generators: `[files.generator]` {#generators}

| Field | Default | Meaning |
| --- | --- | --- |
| `script` | Required string | Category-relative script; its standard output supplies the file content. |
| `runtime` | `native` | [Script runtime](#runtimes). |
| `env` | Required nonempty array | [Inputs](#environment-inputs) supplied to this generator, including its `when_env` source. |
| `when_env` | Required string | Run only when this source name is present in Shine's environment; it must also appear in this generator's `env`. |
| `auto` | `true` | Permit generation during upgrade. With `false`, use explicit installation or refresh; ordinary update/upgrade does not fetch fresh generated content. |
| `fallback` | None | An alternate generator used when the primary `when_env` is absent and its own `when_env` is present. It requires a different trigger and the same `auto` value. Nesting is unsupported. |

Without an enabled generator, Shine uses the source file. Inspection does not run code unless explicitly
requested. Generation still requires approval or trust; failure preserves the last successful
installed content. See [refreshing generated files](../guides/app-presets.md).

### Artifacts: `[artifact]` {#artifacts}

| Field | Default | Meaning |
| --- | --- | --- |
| `script` | Required string | Category-relative apply script. |
| `teardown` | None | Category-relative cleanup script, required for `remove`. |
| `runtime` | `native` | Shared [runtime](#runtimes) for both scripts. |
| `env` | `[]` | [Inputs](#environment-inputs) available to both scripts. |

Run with `shine app artifact apply <CATEGORY>` or `remove`. Scripts define what integration and
cleanup actually do; metadata alone does not provide rollback. See the [artifact workflow](../guides/tasks-and-serve.md#generate-application-helper-resources).

### Lifecycle hooks {#hooks}

`post_install` and `post_upgrade` are category-level fields. Each hook has:

| Field | Default | Meaning |
| --- | --- | --- |
| `command` / `script` | Exactly one required | Program to run, or category-relative script path. |
| `args` | `[]` | Argument strings passed to the entry. |
| `runtime` | `native` for scripts | Only valid with `script`; see [runtimes](#runtimes). |
| `show_output` | `false` | Show successful hook output when the operation reports success output. |
| `env` | `[]` | [Inputs](#environment-inputs) for this hook. |

Trigger timing and failure handling are in [application setup](../guides/app-presets.md#build-helper-resources).
Do not invoke artifact lifecycle commands from hooks.

### Secret files: `[[secret_files]]` {#secret-files}

Requires App metadata version `3` and a workspace version `3` file group.

| Field | Default | Meaning |
| --- | --- | --- |
| `workspace` | Required string | Workspace file path relative to the category. |
| `file_group` | Required string | File group declared in that workspace. |
| `mode` | Required string | Declared workspace mode, such as `production`; no implicit default-mode selection. |
| `auto_decrypt` | `false` | Release and manage plaintext during install/upgrade; `false` installs the encrypted material without automatic plaintext release. |
| `unix_uid`, `unix_gid` | Installing process's user/group | Numeric owner/group for released files on Unix. Changing ownership may require administrator access. |
| `unix_mode` | `"0600"` | Unix permission string, only `"0400"` or `"0600"`. |

Omit all Unix fields on Windows. Workspace mappings supply the plaintext targets. Do not duplicate
workspace/ciphertext files in `files`, include plaintext in the preset, or overlap other managed
targets. Secret files do not support generators, transforms, JSON merge, or automatic target
relocation. Follow [permissions and recovery limits](../guides/app-presets.md#managed-secret-files)
before enabling plaintext release.

### App script paths {#app-script-paths}

Generators, script hooks, artifacts, and teardown receive these variables without `env` declarations;
command hooks do not receive this App-specific set.

| Variable | Meaning |
| --- | --- |
| `SHINE_APP_ID` | Application category name. |
| `SHINE_APP_DIR`, `SHINE_APP_SOURCE_DIR` | Files supplied to this run; also the script working directory. |
| `SHINE_APP_OVERLAY_DIR` | Overlay files for this run, or empty. |
| `SHINE_CONFIG_DIR` | Active Shine configuration and state root. |
| `SHINE_STATE_DIR` | Persistent output directory for this category. |
| `SHINE_CACHE_DIR` | Cache output directory for this category. |

Source directories are temporary: do not save their paths or write persistent results there.
Use the state/cache directories for retained output, creating them when needed.

## Shell categories {#shell}

| Category field | Default | Meaning |
| --- | --- | --- |
| `description` | None | Category description. |
| `permission_defaults` | None | [Permissions](#permissions) for entries without their own table. |
| `files` | Discover native scripts | Explicit command entries. Omission discovers matching `.sh`/`.ps1` scripts; use an explicit list for Bun entries and to exclude helper scripts. |

Each `[[files]]` entry supports:

| Field | Default | Meaning |
| --- | --- | --- |
| `source` | Required string | Existing category-relative script path. |
| `target` | Source filename without its script extension | Installed command name, not a path; command names must be unique within the active category. |
| `description` | Leading script comments | Human-readable command description. |
| `needs_source` | `false` | Expose a native entry through a shell function so it can change the current session. Invalid with Bun. |
| `platforms` | No additional filter | [Platform selectors](#platforms-and-paths); native scripts must also match the shell type. |
| `runtime` | `native` | [Script runtime](#runtimes). |
| `transforms` | `[]` | Ordered [transforms](#transforms). |
| `env` | `[]` | [Runtime inputs](#environment-inputs), supported only for Bun entries. |
| `permissions` | Category `permission_defaults` | Replaces the default table for this entry, without merging it. |

For native scripts, `# shine-template: true` near the beginning also enables template substitution
when the explicit transform list is empty. Bun entries must declare `transforms` explicitly.

Shell categories do not use App's `metadata_schema_version` or Sys's `version` field.

## Sys categories {#sys}

### Category and item fields {#sys-items}

| Category field | Default | Meaning |
| --- | --- | --- |
| `version` | Required: `2` | Sys preset format; missing/version `1` requires migration. |
| `description` | Empty string | Category description. |
| `default_profile` | None | Existing profile used for default selection. When installation items exist, required for noninteractive bootstrap with no explicit items/profile selected. |
| `permission_defaults` | None | [Permissions](#permissions) for items without their own table. |
| `items` | `[]` | Installation or managed-setting items. |
| `profiles.<name>.items` | `[]` | Array of existing installation item IDs. Managed items cannot be included; profiles do not define Shell content or disable other integrations. |

Each `[[items]]` entry supports:

| Field | Default | Meaning |
| --- | --- | --- |
| `id` | Required string | Unique ID using letters, digits, `-`, or `_`. |
| `label` | Required string | Nonempty display label. |
| `description` | Empty string | Item description. |
| `default` | `false` | Preselect an installation item in interactive selection when no `default_profile` exists. |
| `mode` | `init` | `init` installs software with bootstrap; `managed` applies/removes system settings. |
| `requires_admin` | `false` | Declare administrator access; package providers may require it independently. |
| `required_env` | `[]` | Unique names of required Shine environment values; missing or whitespace-only values fail preflight. |
| `permissions` | Category `permission_defaults` | Replace the default permission table for this item. |
| `detect`, `install` | Required for `init` | Detection and installation instructions below; forbidden for `managed`. |
| `shell` | `[]` | Shell integrations below; forbidden for `managed`. |
| `driver` | `script` | `init` uses `script`; managed resources use `split-dns` or `managed-file`. |
| `config` | Empty table | Driver-specific settings; no settings for `script`. |

### Detection and installation {#sys-install}

`[items.detect]` requires one `kind`:

| Kind | Fields and defaults |
| --- | --- |
| `command` | Required `command` name; optional `version_args = []` requests version information from that program. |
| `path` | Required `path`; checks whether the path exists. |
| `any` | Required nonempty `probes` array of `{ kind = "command", command = "..." }` or `{ kind = "path", path = "..." }`; any successful probe is sufficient. |

`[items.install]` requires one `kind`:

| Kind | Fields and defaults |
| --- | --- |
| `package` | Required `provider` and `package`. Providers: `homebrew`/`homebrew-cask` on non-Windows systems, `apt` on Ubuntu, `pacman` on Arch, and `winget` on Windows. Specify one package identifier, not command options. |
| `script` | Required category-relative `path`; installs only this item. |
| Both | Optional `success_status` defaults to `installed`; `needs-action` indicates a user follow-up. Optional `success_hint` defaults to an empty string. |

Detected software is skipped; bootstrap does not upgrade it. Arch requires a full system update
before installation; see [Arch preparation](../built-in-presets/system-bootstrap.md#arch-linux-preparation).

### Shell integration: `[[items.shell]]` {#sys-shell}

`shells` also controls default-list visibility and automatic bootstrap selection: any integration
matching the target shell includes the item; items without integrations are unrestricted. Explicit
item installation bypasses this filter. `when_command`, phase, and installation detection do not
affect selection. These declarations do not extend the platform's [supported shells](../guides/system-init.md#platform-notes).

| Field | Default | Meaning |
| --- | --- | --- |
| `shells` | Required nonempty array | `bash`, `zsh`, or `powershell`. |
| `phase` | Required | `pre` or `post`, before or after the user's main profile content. |
| `priority` | `0` | Lower numbers load first within the phase; ties use declaration order. |
| `when_command` | None | Apply this integration only when that command is available. |
| `path` | None | Add a directory to the command search path. |
| `env` | Empty table | Set literal environment values; this is not the structured secret-input array above. |
| `eval` | `[]` | Execute an argument array and evaluate its output as shell configuration; `{shell}` expands to `bash`, `zsh`, or `pwsh`. |
| `source` | None | Load an existing shell file. |
| `aliases` | Empty table | Map alias names to commands for Bash/Zsh; use a `fragment` for PowerShell. |
| `fragment` | None | Load a category-relative Shell fragment. |

Each integration must choose exactly one of `path`, `env`, `eval`, `source`, `aliases`, or `fragment`.
Executable integration still needs code authorization.

### Managed drivers: `[items.config]` {#sys-managed}

| Driver | Configuration |
| --- | --- |
| `split-dns` | Required strings `domain_env` and `servers_env` name Shine variables containing the DNS suffix and server addresses. Both names must be in `required_env`. |
| `managed-file` | Required `source` is relative to the category; `target` must resolve to an absolute path (`~/` and `$HOME/` are supported). Optional `transforms = []` uses the shared pipeline; optional `restart_hint` displays follow-up instructions. |

Managed items are applied explicitly with `shine sys apply <ITEM>`, not bootstrap profiles.
For prerequisites, removal, and recovery, see [system settings](../guides/system-init.md#managed-system-items).

## Capability declarations {#permissions}

Use `[permissions]` for App, `[files.permissions]` for a Shell entry, or `[items.permissions]` for
Sys. Shell and Sys also accept category-level `[permission_defaults]`; an entry's own table replaces
that default as a whole. Tables are optional, but identified App script behavior must be declared.

| Field | Default | Meaning |
| --- | --- | --- |
| `schema_version` | Required | `1` or `2`; `opaque_code` requires `2`. |
| `administrator` | `false` | Declare administrator access; does not grant it. |
| `opaque_code` | None | Only `"unrestricted"`; declare code whose effects cannot be described completely. Does not bypass App code checks. |
| `filesystem` | `[]` | Entries with required `access`, `base`, and `path`. `access` is a nonempty array of `read`, `write`, `remove`, `execute`. Bases: `home`, `shine`, `data-dir`, `preset`, `absolute`. |
| `network` | `[]` | `{ scope = "host", host = "api.example.com" }` for a host, or `{ scope = "any" }` without a host. The host must not contain a URL scheme, path, credentials, or whitespace. |
| `commands` | `[]` | Program names without arguments. |
| `environment` | `[]` | Entries with required `name` and `sensitivity = "plain"` or `"secret"`; never values. Inputs declared in `env` already supply these declarations. |
| `system` | `[]` | Entries with required `capability` and optional `resource` identifying the system setting. Capability names start with an ASCII lowercase letter or digit, then use those characters plus `.`, `_`, or `-`. |

Filesystem paths are relative to the selected base, except `absolute`; do not use `..`. Relative
paths use `/` separators with no empty or `.` components; `.` alone refers to the base itself. Do not repeat capabilities already expressed by input, command, script, or
destination fields.

**Declarations are not a sandbox or an authorization grant.** Scripts run with their process's
access. Review code and the operation plan, then follow [external-code authorization](../guides/custom-presets.md#authorize-external-code).

Validate the completed category with `shine preset validate <PATH>` and `shine preset lint <PATH>`.
`shine preset schema --format json` describes reports, fixtures, and selected authoring contracts;
it is not a complete schema for every `shine.toml` field.
