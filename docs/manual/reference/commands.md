---
title: Command reference
sidebar_position: 1
---

# Command reference

This page reflects Shine 2.0.0-rc.1. Use `--help` on any subcommand for the exact interface of the
installed version.

## 1.0 target rules

Canonical targets are `app/<category>`, `shell/<category>`,
`shell/<category>/<command>`, and `sys/<item>`. Shell command targets are supported by install and
uninstall; upgrade reconciles the commands already installed in their owning category. Installation
and uninstall also accept a bare category when it is unique across app and shell presets. Bare shell
command names are inspection-only. Prefer complete targets in scripts and documentation to avoid
future ambiguity.

```bash
shine list --available
shine info app/starship
shine install app/starship
shine update
shine upgrade app/starship
```

Since 1.0, `install --replace-managed` replaces `reinstall`. Legacy top-level `clear`, `pull`,
`export`, `link`, and `overlay`, plus `app build/unbuild`, `sys init`, and `env show`, have no
compatibility aliases.

## Top-level commands

| Command | Purpose |
| --- | --- |
| `shine init [--yes]` | Create project `shine.config.toml` in the current directory |
| `shine shell <SUBCOMMAND>` | Manage shell command presets |
| `shine app <SUBCOMMAND>` | Manage application configuration presets |
| `shine install <TARGET> [--replace-managed] [--yes]` | Install or repair an app/shell target |
| `shine uninstall <TARGET> [--force] [--purge] [--dry-run] [--yes]` | Uninstall an app/shell target |
| `shine completions <SUBCOMMAND>` | Generate or install shell completions |
| `shine list [--available [KIND]]` | List installed resources, or browse available `app`, `shell`, and `sys` catalogs |
| `shine info <TARGET> [--diff] [--verbose]` | Inspect an available or installed app/shell target or `sys/<ITEM>` |
| `shine update [TARGET]` | Check managed content and stable Shine updates |
| `shine upgrade [TARGET] [--yes]` | Apply all or selected app, shell, and managed-system updates |
| `shine preset <SUBCOMMAND>` | Manage sources, overlays, exports, and Git synchronization |
| `shine state migrate [--dry-run]` | Migrate and clean legacy runtime state |
| `shine trust <SUBCOMMAND>` | Inspect, grant, list, or revoke target-scoped external-code trust |
| `shine self <SUBCOMMAND>` | Install or upgrade the Shine binary |
| `shine serve <SUBCOMMAND>` | Publish resources under `~/.shine/http/` through a local HTTP service |
| `shine env <SUBCOMMAND>` | Manage preset variables, workspace environments, proxies, and secrets |
| `shine sys <SUBCOMMAND>` | Manage system bootstrap and managed system configuration |
| `shine theme sync` | Detect light/dark terminal appearance and print shell exports |
| `shine ssh ...` / `shine local ...` | Open SSH, broker secrets, and transfer files with POSIX remotes |
| `shine task <SUBCOMMAND>` / `shine run <NAME>` | Save and run personal commands |

Every command accepts global `--config-dir <PATH>` to select the global configuration and runtime
directory temporarily.

## Shell and application presets

```text
shine shell list
shine shell info <CATEGORY|COMMAND|CATEGORY/COMMAND>
shine shell install [<CATEGORY>|<CATEGORY>/<COMMAND>] [--dry-run] [--replace-managed] [--yes]
shine shell recover [--yes]
shine shell uninstall [<CATEGORY>|<CATEGORY>/<COMMAND>] [--purge] [--dry-run] [--yes]

shine app list
shine app info <CATEGORY> [--run-generators] [--diff]
shine app install [CATEGORY] [--dry-run] [--replace-managed] [--yes]
shine app refresh <CATEGORY> [FILE] [--force] [--yes]
shine app recover [--yes]
shine app uninstall [CATEGORY] [--force] [--purge] [--dry-run] [--yes]
shine app artifact apply <APP_ID> [--yes]
shine app artifact remove <APP_ID> [--yes]
```

`--replace-managed` overwrites managed content modified after installation; inspect
`shine info <TARGET> --diff` first. `app uninstall --force` deletes user-modified managed files, so
preview with `--dry-run`. For eligible static Copy files, that forced deletion is journaled and
stages the modified file as same-directory rollback material until receipt commit. Administrator
static Copy paths use the same journaled transaction for creation, in-place update, and removal
through privileged writes, moves, mode restoration, and cleanup. JSON merge install, in-place
update, ordinary uninstall, and forced uninstall are journaled with top-level-key ownership; other
install strategies retain their existing lifecycle path.

`shell install --dry-run` resolves metadata, deployment sources, Bun policy, and intended command
links, but does not extract or snapshot presets, render templates, create links, write a manifest,
or edit shell profiles.

During a command's first installation, Shine journals launcher creation before writing a Unix
symlink, Unix Bun/live launcher, or Windows PowerShell/cmd shim pair. The journal is cleared only
after the exact command receipt is durable, before shell profile editing begins. If this operation
is interrupted, later mutating Shell commands stop with recovery guidance. Run
`shine shell recover` to review a separate recovery Plan. Without a matching receipt it removes
only transaction-created launcher resources whose target or content hash and mode remain exact;
changed paths block recovery and are preserved. If the exact receipt is already durable, recovery
keeps the launcher and clears only stale journal state. Recovery defaults to No and requires
`--yes` outside an interactive terminal.

Install and upgrade also journal an in-place launcher update when the old command receipt and every
launcher resource still match. Changed resources move to canonical same-directory
`.shine.rollback` paths before replacement. Before the new receipt is durable, recovery restores
only exact previous resources; after receipt commit, it keeps exact replacements and removes only
unchanged rollback material. Any changed replacement, rollback resource, or conflicting receipt
blocks recovery. Foreign or already modified launchers do not inherit this rollback proof.

Approved uninstall journals launcher removal only when the old receipt and every reconstructed
launcher resource still match. Each Unix launcher or Windows shim moves to its same-directory
`.shine.rollback` before receipt removal. After receipt removal, a separate durable journal marker
must confirm commit before rollback cleanup. If the receipt was removed but that marker was not
written, `shine shell recover` recreates the old receipt before restoring exact resources. Once the
marker is durable, recovery keeps the completed uninstall and removes only unchanged rollback
material. A changed launcher, rollback path, or conflicting receipt blocks recovery and is
preserved.

For external presets in snapshot mode, install and upgrade also journal a changed shared category
snapshot when the selected commands require no rendered output. The action uses deterministic
category sibling stage/rollback directories and a positive commit marker independent of receipt
equality. Before that marker, `shine shell recover` restores the previous selected receipt set
before assessing dependent launchers, then restores the exact old tree. After the marker it keeps
the desired tree and removes only exact rollback. Changed active, stage, or rollback trees block
recovery. Snapshot uninstall uses its own removal action with the same receipt/marker boundary.

For embedded presets, install journals actual category cache writes before dependent rendered-file
or launcher changes. Missing files and differing files changed by upgrade or `--replace-managed` each bind
previous/desired hash and mode plus same-directory rollback; skipped and unrelated cache files stay
outside the action. Before its positive marker, recovery restores previous receipts and exact old
files or removes exact created files. Afterward it keeps desired files and cleans exact rollback.
A non-file destination, occupied/modified rollback, modified cache file, or receipt conflict blocks
the whole cache action. Cache uninstall uses a removal action that restores only exact selected
files and receipts before its positive marker.

When install or upgrade creates or changes transformed output, Shine also journals the rendered
file before dependent launcher changes. An existing file moves to its canonical same-directory
`.shine.rollback`; the journal binds its previous and desired hash/mode, every consuming command
receipt transition, and a positive commit marker. Before that marker, recovery restores the
previous receipts and exact prior file, or removes an exact transaction-created file. After the
marker it keeps the desired file and cleans only exact rollback. A changed or non-file destination,
occupied or modified rollback, or conflicting receipt blocks recovery. Uninstall also journals a
rendered file when every consuming receipt is selected: the exact file moves to rollback before
receipt removal, and receipt absence requires a positive marker before cleanup. Before that marker,
recovery reconstructs missing receipts and restores the exact file; afterward it preserves absence
and cleans exact rollback. Unselected consumers and unrelated rendered files remain untouched.
Execution-time live rendering shares the lifecycle/recovery lock and refuses a pending journal while
remaining invocation-scoped and atomic. Profile reconciliation uses a separate sentinel-owned
action. Recovery merges only the recorded `# >>> shine >>>` block transition into the current
profile and preserves unrelated edits.

Without `--dry-run`, App and Shell lifecycle mutations, App refresh, and artifact apply/remove
display a snapshot-bound security Plan and ask once with a default answer of No. `--yes` still
prints and revalidates the full Plan but skips the prompt; redirected or otherwise non-interactive
execution must use it. `--yes` and `--dry-run` are mutually exclusive where dry-run exists. Dry-run
retains its existing preview format and is not an approved Plan.

`app refresh` handles only generated files tracked by the manifest and preserves the last successful
content on failure. Artifact apply/remove explicitly runs an external integration declared by the
preset; ordinary installation and upgrade do not implicitly apply it.

If a supported App creation, in-place static Copy update, or ordinary removal of an unchanged
static Copy is interrupted after its operation journal is written,
later mutating App commands that require a security Plan stop with recovery guidance instead of
changing that state implicitly. Read-only inspection does not recover or discard the journal. Run `shine app recover`
to inspect a separate recovery Plan. It preserves files that have changed since the interruption;
for a backup-aware creation it restores the fixed backup only while both destination and backup
still match the journaled original/desired fingerprints. When the ownership receipt is already
durable it keeps the managed destination and persistent backup and clears only stale transaction
state. An in-place managed update temporarily moves the prior managed file to
`<name>.shine.rollback`; recovery restores or removes that file only while it still matches the
journaled prior fingerprint and mode. For ordinary supported removal, the old receipt causes
recovery to restore unchanged rollback material; once receipt removal and its journal commit state
are durable, recovery removes only that unchanged material. Receipt absence without the matching
journal state instead recreates the old receipt and restores the unchanged file.
For a backup-restoring removal, Shine first moves the managed file to `.shine.rollback` and then
moves `.shine.bak` to the destination. Recovery before receipt commit reverses only an exact safe
state of those three paths, restoring both the managed destination and persistent backup. Recovery
after commit keeps the exact restored user destination and removes only unchanged managed rollback
material. Both file modes and hashes must still match the journal.
Forced removal of a user-modified static Copy uses a distinct action: recovery before
receipt commit restores the exact modified file and reverses an optional backup restoration;
recovery after commit keeps the completed uninstall and removes only rollback material matching
the captured modified mode and hash.
JSON merge recovery uses an exact whole-file rollback only as the source of previous declared-key
values. It restores or removes those keys in the current object without replacing unrelated values
changed after interruption. After uninstall receipt commit, it preserves the user-owned current
object and removes only exact rollback material.
Administrator static Copy recovery includes administrator permission only when the exact recovery
state requires a protected path write, move, removal, or mode change. Shine requests that
authorization after recovery Plan approval; receipt-only repair and stale-journal cleanup do not
request it.
Treat interrupted rollback material as sensitive managed configuration. Recovery
defaults to No and requires `--yes` when no interactive terminal is available. A missing or invalid
journal, unsupported action, or changed destination/backup/rollback path returns nonzero without
mutation. An existing fixed backup or update/removal rollback path blocks the corresponding
supported Plan instead of being replaced.

## Status, updates, and completions

```text
shine list [--available [<app|shell|sys>]]
shine info <TARGET> [--diff] [--verbose] [--run-generators]
shine update [TARGET] [--pull] [--diff] [--verbose] [--refresh-release] [--run-generators]
shine upgrade [TARGET] [--pull] [--verbose] [--prune-stale] [--yes]
shine state migrate [--dry-run]
shine trust inspect <app/CATEGORY|sys/ITEM>
shine trust grant <app/CATEGORY|sys/ITEM> [--yes]
shine trust list
shine trust revoke <app/CATEGORY|sys/ITEM>
shine completions install
shine completions <bash|zsh|powershell>
```

Trust enrollment derives its scope from the current immutable Preset snapshot. `--yes` confirms
the rendered enrollment non-interactively; it does not approve later lifecycle Plans.

App generators are never executed by default during `app info`, top-level `info`, or `update`.
When generated desired content cannot be determined statically, these commands display a prominent
not-evaluated warning instead of claiming that the installed file is current. Pass
`--run-generators` to explicitly execute automatic and manual generators, apply transforms in
memory, and calculate status or `--diff` output without writing destinations or manifests. Global
`update --run-generators` evaluates every installed App category; targeted info/update evaluates
only the selected App. External generators still require a matching `shine trust grant`, and
evaluation failures are reported after the remaining selected generators run.

- `update --refresh-release` bypasses the 24-hour cache. By default, `update` groups targets under
  the same Homebrew-style sections as `shine list`: interactive terminals use horizontal columns,
  while redirected output stays one target per line. When exactly one category or managed-system
  item needs an update, the final hint uses its canonical target, such as
  `shine upgrade app/clash-verge`; multiple targets keep the aggregate `shine upgrade` hint. App
  files and Shell commands collapse to their category. `update --diff` switches to detailed
  vertical rows and expands affected files and commands. Structural changes such as source or
  destination relocation, new files, deployment metadata, and command-entry refreshes are shown
  field by field; a unified diff is
  printed only when content changed. Targeted `update <TARGET>` uses the same details.
  For structural-only updates, Shine identifies a missing or mismatched command entry and a missing
  Shell manifest record separately, then prints `content: unchanged` instead of an empty diff.
  A targeted `update <TARGET>` is already detailed, so adding `--diff` changes only an untargeted
  update from category summaries to expanded rows.
- Inline diffs require valid UTF-8 text without NUL bytes and are limited to 256 KiB per side.
  Binary, invalid UTF-8, and larger content is summarized with byte counts instead of being dumped
  to the terminal. `info --diff` uses the same protection.
- A targeted update can accept `--verbose` for command-line compatibility, but targeted output is
  already detailed, so the flag does not add more rows. It cannot combine with `--refresh-release`
  because targeted checks do not perform a Shine release check.
- `update/upgrade --pull` synchronizes Git-managed sources and reloads configuration first.
- Untargeted `upgrade` reviews Shell, App, and enabled managed-system Plans together, confirms once,
  and revalidates all of them before applying changes. It no longer changes Sys profile enablement
  or composition implicitly. Its required permissions and missing-declaration checks include only
  installed App categories, installed Shell commands, and enabled managed-system items; merely
  available Presets that have not been installed do not contribute to the Plan. Shell category
  cache or snapshot work is included only when that category has a selected installed command or
  compatible legacy managed launcher, and a fully current command contributes no command-local
  mutation permissions. Embedded Shell cache permissions follow the current OS and shell's
  effective command sources: Bash/Zsh plans do not request writes for native `.ps1` sources, and
  PowerShell plans do not request writes for native `.sh` sources. Category metadata and unbound
  shared helper files remain cached.
- By default, `upgrade` renders that review as one compact Plan with separate Shell, App, and System
  sections. No-op steps are counted, consecutive per-category Preset-cache steps are summarized,
  permissions are grouped by capability while retaining every reviewed identity, and snapshot/
  Plan identities are shortened for display. Preserve and blocked steps remain explicit. Pass
  `--verbose` to print every ordered step and full digest/fingerprint instead. Missing declarations
  and untrusted external App code produce actionable guidance; either condition still blocks the
  whole reviewed batch before its first mutation. Lifecycle action markers and status words use
  restrained semantic colors—green create, yellow update/preserve, red remove/blocked, cyan execute,
  and dim unchanged—while targets and permission identities remain plain. Redirected output and
  terminals with color disabled receive the same text without ANSI escapes.
- `upgrade --prune-stale` removes unchanged managed App entries no longer present in the source
  through the App operation journal. User-modified stale content remains preserved; interrupted
  removal is handled by `app recover`.
- An App static Copy whose effective destination changes is relocated through one journaled
  old-receipt/new-receipt transaction. The old managed content must be unchanged and the new path
  absent; interrupted relocation is handled by `app recover`.
- An App JSON merge whose effective destination changes uses a key-owned two-destination
  transaction. Recovery restores/removes only the old/new declared top-level keys and preserves
  unrelated current settings at both paths.
- After approval, `upgrade` prints each app category, Shell category, or managed-system item it actually
  updates and counts each user-facing target once. App rows include the number of changed files.
  `--verbose` expands app files and successful hook output, and also shows current/skipped items and
  Shell deployment details such as snapshots, templates, and Bin Links. Failures, conflicts,
  user-modified warnings, and blocked hooks remain visible without `--verbose`.
- `shell info` and top-level `info` inspect uninstalled presets; `list --available` filters by kind.
- Default list, update, and upgrade summaries use category-level lifecycle identities.
  `info`, `--diff`, and verbose deployment sections retain file, command, link, and receipt details.

## System presets

```text
shine sys list [--all]
shine sys info <ITEM>
shine sys status
shine sys recover [--yes]
shine sys bootstrap [ITEM]... [--item <ITEM>]... [--preset <PROFILE>] [--dry-run] [--force-profile] [--proxy] [--yes]
shine sys profile enable <ITEM> [--dry-run] [--yes]
shine sys profile disable <ITEM> [--dry-run] [--yes]
shine sys apply [ITEM] [--dry-run] [--yes]
shine sys uninstall <ITEM> [--dry-run] [--yes]
```

Positional items, repeated `--item`, and `--preset` are mutually exclusive. Before mutation,
`sys bootstrap` renders a snapshot-bound security Plan and asks for default-No approval. Use
`--yes` for non-interactive approval; it still renders and revalidates the Plan and conflicts with
`--dry-run`. Bootstrap ensures only the selected software is present and enables its declared
shell integration; rerunning it never upgrades the software. `sys profile enable/disable` uses the
same Plan approval contract and changes only Shine-owned integration content. Use the
software's own package manager or upstream tool for upgrades; `shine upgrade sys/<ITEM>` converges
an independent managed item.

Managed-file and split-DNS mutations, plus the shell sentinel changes made by explicit
`sys profile enable/disable`, are journaled before resource mutation and committed only after the
exact Sys receipt is durable. A pending journal blocks later mutating Sys commands. Run
`shine sys recover` to review a fresh recovery Plan; it restores only fingerprint-matching previous
state before receipt commit, or keeps desired state and cleans exact rollback afterward. Changed
resources, rollback material, owned sentinel blocks, or receipts block recovery and are preserved.
Generated active/base/new/merge profile files retain their three-way merge behavior and are shown
as non-transactional; bootstrap scripts and package/provider calls remain explicitly opaque and
outside this recovery boundary.

## Preset sources and customization

```text
shine preset new <app|shell|sys> [--force]
shine preset schema [--format <text|json>]
shine preset validate [PATH] [--format <text|json>]
shine preset lint [PATH] [--format <text|json>] [--deny-warnings]
shine preset plan <CATEGORY> --platform <macos|linux|windows> [--format <text|json>]
shine preset test <CATEGORY> [--format <text|json>]
shine preset pack <CATEGORY> --output <FILE> [--force] [--format <text|json>]
shine preset export [DIR] [--force]
shine preset copy <app|shell|sys>/<NAME> [--force]
shine preset link <PATH> [--create] [--live]
shine preset unlink
shine preset overlay link [<PATH> | --git <URL> [--branch <BRANCH>]] [--create]
shine preset overlay info
shine preset overlay unlink
shine preset pull
```

`preset copy` copies one complete built-in category for a partial overlay; `preset export` exports the
full collection. External shell presets use snapshots by default and require `shine upgrade` after
source changes. `--live` is for preset development.

`preset schema` generates reference schema v1 from the report, fixture, and bundle Rust types
shipped in the current binary, plus the live Clap help for `validate`, `lint`, `plan`, `test`,
`pack`, and `schema`. Text output lists the included contracts; `--format json` emits command help
and JSON Schema draft 2020-12 documents in one JSON value. It does not duplicate the complete
App/Shell/Sys TOML grammar: `preset validate` remains the metadata acceptance authority. The
command does not load or initialize configuration.

`preset validate` accepts a preset repository root, an `app|shell|sys/<name>` category, or its
`shine.toml`; the path defaults to the current directory. It statically checks every declared
platform branch and referenced file without loading the active preset source, initializing Shine
configuration, checking for updates, accessing the network, or running preset code. Invalid input
or categories exit with status 1; warnings do not. JSON output uses `schema_version: 1` and contains
no colors or explanatory text outside the JSON document. See
[Customize presets](../guides/custom-presets.md).

`preset lint` accepts the same repository, category, and manifest inputs as validation, reuses the
validated immutable metadata, and reports author-quality or portability findings without changing
runtime validity. Report schema v1 covers missing category/resource descriptions, legacy metadata,
broad `network any` declarations, and absolute permission/destination paths that appear to contain
a private machine HOME. It reports only logical targets and resources, never the suspected private
path. Warnings do not fail by default; `--deny-warnings` exits with status 1 when the otherwise valid
report is not clean. Static validation errors always fail.

`preset plan` accepts exactly one category directory or its `shine.toml`. It first reuses static
validation, then builds a hypothetical first-install report for the selected platform against an
empty in-memory host. The assumptions intentionally contain no installed receipts, destinations,
environment or secret values, trust grants, detected commands, or administrator state. App and
Shell categories show their install steps; Sys categories show separate managed-resource and
bootstrap sections when applicable. The command never initializes configuration, touches the real
HOME, runs preset code, or produces an approval that can be applied. `ready: false` describes a
blocker under the stated assumptions and does not make an otherwise valid report fail; invalid
input or static validation still exits with status 1. JSON output uses its own `schema_version: 1`.

`preset test` reads `shine.test.toml` from exactly one category and runs each declared case through
the same synthetic authoring-plan path. Fixture schema v1 requires unique case names and a platform.
Optional `[cases.host]` state may declare environment-name presence, opaque `secret_versions`,
synthetic files under `home|shine|data-dir|bin|absolute`, detected command names, administrator
state, exact external-code trust selections, and App/Shell/Sys receipt documents. Receipt text may
use `${HOME}`, `${SHINE}`, `${DATA_DIR}`, and `${BIN}` placeholders and must parse as the current
runtime manifest schema. Fixture values never enter reports.

`[cases.expect]` may assert `valid`, `ready`, and exact sorted sets named `plan_kinds`,
`diagnostic_codes`, `step_diagnostic_codes`, `actions`, `required_permissions`,
`missing_permissions`, and `permission_diagnostic_codes`. Missing expectation fields are not
asserted; an explicit empty list asserts no values. The JSON case result includes all corresponding
actual sets to support repair loops. Fixtures cannot declare setup, teardown, commands to run,
network activity, or executable code. A parse/schema error or any failed case exits with status 1,
and JSON report schema v1 contains stable failure codes rather than terminal prose.
Permission identities use `administrator`, `command:<program>`, `network:any`,
`network:host:<host>`, `environment:<plain|secret>:<name>`,
`filesystem:<read|write|remove|execute>:<logical-path>`, or
`system:<capability>[:<resource>]`.

`preset pack` validates one category and atomically writes a deterministic bundle outside that
category. Bundle v1 is an unsigned tar.gz containing `shine.bundle.json` plus sorted files under
`preset/<kind>/<name>/`; the manifest records logical paths, normalized `0644`/`0755` modes, and
SHA-256 values. Checkout roots, enumeration order, uid/gid, timestamps, and `shine.test.toml` do not
affect bytes. Packing rejects `node_modules`, symlinks, private-key filenames/material, private HOME
paths, and executable/shebang files not referenced by metadata without printing the suspected data.
An existing output requires `--force`; output inside the category is always rejected. Report schema
v1 includes the final archive size and SHA-256. Signing and registry publishing are not part of this
command.

## Environment values and secrets

```text
shine env list [--reveal]
shine env set <KEY> <VALUE> [--force]
shine env get <KEY>
shine env delete <KEY> [--force]
shine env run [--workspace <FILE>] [--mode <MODE>] [--no-workspace] [--with <KEY[=ALIAS]>]... [--secret-broker [--secret <KEY[=ALIAS]>]...] -- <COMMAND>...
shine env workspace init --from-dotenv [--mode <MODE>]... [--secret <KEY>]... [--force] [--dry-run]
shine env workspace export --format dotenv [--workspace <FILE>] --mode <MODE> --output <FILE> [--include-secrets] [--force] [--dry-run]
shine env broker describe [--workspace <FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy <add|update> --name <NAME> --ssh-target <TARGET> [--project <PROJECT>] --workspace <FILE> [--remote-workspace <REMOTE_FILE>] --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy diff <NAME> --workspace <FILE> --mode <MODE> (--release <KEY>... | --release-all-declared) -- <COMMAND>...
shine env broker policy list
shine env broker policy info <NAME>
shine env broker policy remove <NAME>
shine env proxy install <COMMAND> --with <KEY[=ALIAS]>... [--project]
shine env proxy list
shine env proxy uninstall <COMMAND>
shine env proxy enable <COMMAND> [--project]
shine env proxy disable <COMMAND> [--project]
shine env secret encrypt [--backend <gpg|age>] [-r <RECIPIENT>]... [--from <KEY>] [--set <KEY>] [--force]
shine env secret decrypt <KEY>
shine env secret export <KEY> [--as <ALIAS>]
shine env secret seal [FILE] [--workspace <FILE>] [--backend <gpg|age>] [-r <RECIPIENT>]...
shine env secret identity init [--touch-id] [--access-control <POLICY>] [-o <PATH>] [--force]
shine env secret identity list
```

`--with` is repeatable and accepts `KEY=ALIAS`. `--no-workspace` uses explicit values and the process
environment only and conflicts with `--workspace` and `--mode`. Workspace initialization currently
requires `--from-dotenv` and supports `--dry-run`. Workspace export requires an explicit format,
mode, and output path. It exports only resolved plain values unless `--include-secrets` is present;
it never includes inherited process values. Broker policy creation chooses one or more explicit
`--release` keys or freezes every currently declared key with `--release-all-declared`; the forms
are mutually exclusive. Touch ID identities are macOS-only and require `age-plugin-se`.

For broker policies, `--project` stores a human-readable project label. `--remote-workspace`
requires remote requests to report that exact absolute workspace path in addition to matching the
workspace contents and other policy fields.

An environment proxy creates a same-name shim under `~/.shine/bin/` and injects only declared values
into its child, preferring `<KEY>_SECRET` over `<KEY>`. Disable retains the shim without injection.
Project rules require a discoverable `shine.config.toml` and override same-name global rules.
Uninstall removes the managed shim and user-level rule.

## Tasks, local service, and theme

```text
shine task save <NAME> [--force] [--cwd <PATH>] -- <COMMAND>...
shine task run <NAME> [-- EXTRA_ARGS...]
shine task list
shine task info <NAME>
shine task delete <NAME>
shine run <NAME> [-- EXTRA_ARGS...]

shine serve install [--port <PORT>]
shine serve start [--port <PORT>]
shine serve status
shine serve uninstall
shine serve url <PATH> [--port <PORT>]

shine theme sync [--auto] [--quiet]
```

Tasks store and execute argument arrays without a shell. `--cwd` fixes the working directory; without
it the caller's directory is used. `serve install` uses launchd on macOS, a systemd user unit on
Linux, and a current-user scheduled task on Windows; `start` runs the local service in the foreground.

## SSH, secret brokering, and transfer

```text
shine ssh [--remote-shell <posix|windows>] [--with <KEY[=ALIAS]>]... [--with-secret <KEY[=ALIAS]>]... [--secret-broker [--allow-secret <KEY[=ALIAS]>]... [--secret-broker-policy <FILE>]... [--trust-remote-session]] [SSH_ARGS]... <HOST> [COMMAND]
shine ssh --secret-broker-inspect <HOST>
shine ssh --secret-broker-enroll --trust-remote-metadata [--update-policy <NAME>] <HOST>
shine local download <REMOTE_SOURCE> [LOCAL_DESTINATION] [--force] [--dry-run] [--scp]
shine local upload <LOCAL_SOURCE> [REMOTE_DESTINATION] [--force] [--dry-run] [--scp]
shine local status
```

Shine options must precede the SSH target. Remote on-demand requests use
`shine env run --secret-broker`; see the [SSH guide](../guides/ssh-transfer.md#provide-secrets-to-remote-commands-on-demand).
A Windows remote uses `--remote-shell windows` for PowerShell injection only, without transfer or
Secret Broker support.

## Program installation and upgrades

```text
shine self install [--dest <PATH>]
shine self upgrade [--channel <stable|preview>]
```

The RC `shine --version` output is `shine 2.0.0-rc.1 (<commit> <date>)`; preview builds use the
SemVer-compatible label `2.0.0-rc.1.preview`.
