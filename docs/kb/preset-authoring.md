# Preset Authoring

Internal authoring guide for embedded and external Shine presets. Public command usage belongs in
the bilingual manual; design rationale belongs in ADRs; behavioral safety rules belong in
[`architecture/invariants.md`](architecture/invariants.md).

## Shared workflow

1. Prefer `shine.toml` metadata over legacy source annotations for new presets.
2. Keep commands, identifiers, and platform constraints explicit in metadata.
3. Treat external preset and overlay code as untrusted unless the corresponding config permission
   has a matching target-scoped trust grant.
4. Keep generated output deterministic. Never print credentials, source URLs containing secrets, or
   raw subscription records in diagnostics.
5. `cli/build.rs` must retain `cargo:rerun-if-changed=presets`; a normal Cargo rebuild then
   re-embeds changed assets.
6. For a user-visible preset change, update the matching English and Simplified Chinese manual
   pages in the same change.
7. Use `shine preset schema --format json` when tooling needs shipped authoring report, fixture, or
   bundle contracts; never maintain a handwritten copy of those generated schemas.
8. Run `shine preset validate <path> --format json` before runtime-specific checks. It validates
   repository roots, category directories, and manifests without loading config or executing code.
9. Run `shine preset lint <path> --format json` and review its author-quality and portability
   findings. Use `--deny-warnings` only for deliberately clean CI policy.
10. Run `shine preset plan <category> --platform <platform> --format json` for every supported target
   platform. Treat it as a hypothetical empty-host report, never as an approval or dry-run.
11. Put repeatable structured assertions in category-local `shine.test.toml` and run
    `shine preset test <category> --format json`. Fixtures may contain bounded synthetic observations
    but never executable setup/teardown, actual credentials, or private machine paths.
12. Build distributable bytes only with `shine preset pack`, outside the category. Fix every policy
    diagnostic; `--force` controls output replacement only.
13. Keep schema-v1 permission declarations at the execution target boundary: one App category
   table, one table per Shell file/platform variant, and one table per Sys item. Declare identities
   only; never place argv, values, ciphertext, credentials, or physical checkout paths in them.
14. After changing a built-in App destination or App/Shell file selector, run
   `SHINE_UPDATE_PRESET_CAPABILITIES=1 cargo test built_in_preset_platform_capability_docs_are_current`
   and commit both regenerated public-manual blocks.

For a 1.x source, begin with `shine preset migrate <path> --dry-run`. The reviewed migrator may
rebase only exact released metadata or apply safe structural App edits; it never supplies opaque
permissions, splits a Sys dispatcher, grants trust, or edits payloads. Complete every reported
manual blocker, then rerun validate, lint, plan, and fixture tests above.

## AI authoring boundary

`skills/shine-preset-author/` is the portable author workflow. Keep `SKILL.md` concise and route to
only one of `references/app.md`, `references/shell.md`, or `references/sys.md`. The skill must treat
the installed CLI as authoritative: generate `preset schema --format json` when available, scaffold
with `preset new` or `preset copy`, require JSON validation/lint and
one hypothetical plan per target platform, and run only isolated dry-runs under a temporary
`SHINE_CONFIG_DIR`. It must never link a source/overlay, activate a preset, or run real install,
upgrade, bootstrap, hook, generator, or artifact actions.

`cli/src/preset_validation.rs` owns path discovery and the schema-v1 report. Domain rules stay with
`apps/metadata.rs`, `shells/metadata.rs`, and `sys/manifest.rs` so runtime and static validation do
not become independent schemas. The validator is routed before `Config::load_or_init()` and must
remain free of config initialization, update checks, process execution, network access, and writes.
Validate the effective `macos`, `linux`, and `windows` branches on every host, including any
declared `unix` compatibility fallback. New diagnostic codes are API surface; keep them stable
within schema version 1.

The skill directory is distributed in the crate but is not embedded as runtime presets. Validate
its Agent Skills frontmatter and directory-name parity after edits. See
[ADR 0033](decisions/0033-skill-first-ai-preset-authoring.md).

## Shell preset category

Create `presets/shell/<category>/shine.toml` and the source files it declares. A minimal native
entry is:

```toml
description = "What this category does."

[[files]]
source = "your_script.sh"
target = "mycommand"
needs_source = false
```

- Native Unix scripts normally use a `#!/bin/bash` shebang. When no explicit metadata description
  is present, Shine can parse the leading `# ` comment block immediately after the shebang.
- `needs_source = true` exposes a shell function instead of a direct launcher; use it only when the
  command must mutate the parent shell.
- `platforms` accepts exact `macos`, `linux`, and `windows` selectors; `unix` is the compatibility
  group for macOS and Linux. Exact and group selectors are ORed, and an empty array is invalid.
- Only `[[files]]` entries become commands. Sibling helper modules are deployment material, not
  activation receipts.

### Bun shell entries

A cross-platform TypeScript/JavaScript command may use:

```toml
[[files]]
source = "command.ts"
target = "command"
runtime = "bun"
```

Rules:

- Bun is an external prerequisite. Shine checks for it but never installs it.
- `runtime = "bun"` accepts `.ts`, `.js`, `.mts`, and `.mjs`; it cannot combine with
  `needs_source = true` because a subprocess cannot modify its parent shell.
- The installed command is a Shine-managed regular launcher, not a symlink. Ownership and removal
  depend on its marker and target contract; see the Bun launcher invariant.
- Use a leading `//` block or the metadata `description` field. Explicit metadata wins.
- Static substitution remains opt-in through `transforms = ["template"]`; JS/TS does not support
  the shell-only `# shine-template: true` annotation.
- Embedded Bun scripts must remain self-contained: only relative modules, `node:*`, `bun`, and
  `bun:*` imports are allowed. `bun run check:ts` scans production preset imports for this rule.
- External/overlay scripts may use registry dependencies when their physical category root contains
  a committed `package.json` and `bun.lock`. The pair is required, any `trustedDependencies`
  declaration is rejected, and local `node_modules/` trees are never copied.
- Shine uses `--no-install` without that pair and `--install=fallback` with it. It does not run
  `bun install` or own Bun's cache; the author must generate and verify the lock in preset CI.

A Bun entry may request runtime env injection:

```toml
env = ["KEY", "SOURCE=TARGET"]
```

The ordered mapping uses the `shine env run --with` grammar. Names are validated and duplicate
targets are rejected at metadata load. At launch the managed wrapper runs through
`shine env run --no-workspace --with ... -- bun <script>`; `KEY_SECRET` is decrypted per invocation.
This requires both `shine` and `bun` on `PATH`. Runtime `env` and static `template` transforms are
independent and may be combined. See
[`../bun-shell-preset-env-injection-prd.md`](../bun-shell-preset-env-injection-prd.md).

### Shell verification

For an isolated plan check, copy the category into a temporary external preset tree and use the
side-effect-free install dry-run:

```bash
cargo build --target-dir target
cargo run --target-dir target -- shell list
cargo run --target-dir target -- shell info <category>
mkdir -p .tmp-home/.shine/presets/shell
cp -R presets/shell/<category> .tmp-home/.shine/presets/shell/
env SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine cargo run --target-dir target -- shell install <category> --dry-run
```

The dry-run validates deployment inputs and prints intended links without extracting, snapshotting,
rendering, linking, recording a manifest, or editing profiles. Add targeted metadata, launcher, and
install/uninstall round-trip tests for new behavior.

## App preset category

Create `presets/app/<category>/shine.toml`. Prefer metadata over the legacy `shine-dest:` source
annotation:

```toml
description = "Managed application configuration."
dest = "~/.config/example"

[[files]]
source = "config.jsonc"
transforms = ["jsonc-to-json", "template"]
```

- A per-file `dest` overrides the category destination. Structured platform data destinations and
  privileged destinations must use the supported metadata model; do not reproduce destination
  resolution in an artifact script.
- `jsonc-to-json` strips comments; `template` replaces `@@VAR_NAME@@` from the active `[env]` table.
  Transforms compose in declaration order.
- `@@VAR@@` is the delimiter for every file type. Quote YAML placeholders when they begin a scalar;
  native-typed YAML env rendering is not supported. See
  [ADR 0013](decisions/0013-template-delimiter-policy.md).

### Lifecycle hooks

Categories may declare `post_install` and `post_upgrade` as a command table or array of command
tables. Both run direct argv commands only when the category actually changed:

- `post_install` runs after an install that writes at least one file, including
  `--replace-managed`.
- `post_upgrade` runs after an upgrade that writes or installs at least one file.
- External preset/overlay hooks require `shine trust grant app/<category>` after review.
- Command hooks receive only their declared `[env]` mappings in addition to the process baseline;
  they do not receive the fixed `SHINE_APP_*` contract.
- `show_output = true` prints successful stdout; otherwise success is quiet.

A hook may instead declare `script` plus an optional `runtime = "bun"`. Script hooks are resolved
from the same immutable Preset snapshot as generators and artifacts, receive their declared `env`
mapping plus the fixed `SHINE_APP_*` contract, and execute inside the parent lifecycle Plan. The
script path needs a Preset `execute` permission and Bun needs a command declaration. `command` and
`script` are mutually exclusive; `runtime` is valid only with `script`. Never use a command hook to
recursively launch `shine app artifact apply`: that mutation owns a separate Plan. Command-hook
inputs remain required; script-hook inputs follow artifact semantics and may be absent, in which
case the Plan binds the missing state and execution omits the variable.

Use a command hook for a direct reload/setup command, a script hook for a snapshot-bound lifecycle
script, and an artifact when the action must remain an explicit user invocation.

### Generated files

An app file may declare:

```toml
generator = {
  script = "generate.ts",
  runtime = "bun",
  env = ["SOURCE_URL"],
  when_env = "SOURCE_URL",
  auto = false,
}
```

- A static `source` remains mandatory as fallback and stable manifest identity.
- When enabled, UTF-8 stdout becomes the effective source before normal transforms and install
  strategies.
- `auto` defaults to true. Automatic generators may run during approved install/upgrade, but never
  during read-only status/update.
- `auto = false` keeps implicit status local-only and preserves the installed snapshot during
  upgrade. Install still generates; `shine app refresh <category> [source] [--force]` is the
  explicit refresh path.
- Only declared `generator.env` values are injected; `_SECRET` values are not decrypted.
- External preset/overlay generators require target-scoped trust and are deadline/output
  limited.
- Failure preserves an existing managed destination as last-known-good. A first-time enabled
  generator failure is fatal.

See [ADR 0016](decisions/0016-generated-app-files-and-surge-subscriptions.md) and
[ADR 0018](decisions/0018-manual-app-generator-refresh.md). Bun generators follow the same locked
external dependency convention as Shell entries; the physical category supplying the generator
script owns the declaration.

### Artifacts

An app category can expose explicit artifact commands:

```toml
[artifact]
script = "build.ts"
teardown = "unbuild.ts"
runtime = "bun"
env = ["PROFILE_PATH", "API_TOKEN"]
```

- `runtime` is `native` by default. Native executes the file directly and relies on its shebang;
  Bun accepts `.ts`, `.js`, `.mts`, or `.mjs` and requires Bun on `PATH`.
- `script` runs only through `shine app artifact apply <app-id>` unless a preset explicitly invokes
  that command from a lifecycle hook.
- `teardown` runs through `shine app artifact remove <app-id>` and best-effort before app uninstall.
- Explicit artifact commands require a reviewed security Plan and propagate nonzero exit. Implicit
  uninstall teardown is included in the lifecycle Plan, remains non-fatal, and is safely skipped
  when external code is not allowed.
- `env` is an explicit source or `SOURCE=TARGET` allowlist. Scripts receive only that allowlist plus
  the fixed `SHINE_APP_*` contract; every source must also appear in the category's
  `[permissions].environment`. Plain values are Plan-bound by hash; secret-classified names require
  opaque versions and are not decrypted by artifact execution.
- An overlay script wins only when that exact artifact path exists; otherwise the active base
  preset script remains available.
- Bun artifacts and teardown use the same locked external dependency convention. Package metadata
  in an overlay does not affect an artifact inherited from the embedded category.

See [ADR 0009](decisions/0009-app-artifact-build-explicit-command.md),
[ADR 0012](decisions/0012-app-lifecycle-post-install-and-teardown.md),
[ADR 0045](decisions/0045-specialized-app-and-profile-security-plans.md), and the
[app artifact data flow](architecture/data-flows.md#app-artifact-build-shine-app-artifact-apply-app-id).

### App verification

Be deliberate about preset mode. With `SHINE_CONFIG_DIR` set, copy the category under test to
`$SHINE_CONFIG_DIR/presets/app/<category>/`; unset external preset settings when verifying embedded
assets.

```bash
cargo run --target-dir target -- app list
cargo run --target-dir target -- app info <category>
cargo run --target-dir target -- app install <category> --dry-run
```

Add targeted tests for destination resolution, metadata parsing, transforms, generators, hooks, or
artifact behavior as applicable. TypeScript presets must pass `bun run check:ts`.

## Sys preset

Create `presets/sys/<os_id>/shine.toml` with `version = 2`:

```toml
description = "One-line OS preset description."
default_profile = "recommended"
version = 2

[[items]]
id = "neovim"
label = "Neovim"
description = "Install Neovim"

[items.detect]
kind = "command"
command = "nvim"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew"
package = "neovim"

[profiles.recommended]
items = ["neovim"]
```

- Every init item declares both `detect` and `install`. Unknown versions and v1 manifests fail
  before execution.
- Detection kinds are `command`, `path`, or `any`.
- Package providers are fixed ensure-present Homebrew, Homebrew Cask, APT, or Winget actions; they
  do not implement package upgrades. Complex installation uses one item-local script with normal
  exit status.
- Put OS-wide shell setup in `profile/base.pre.*` and `profile/base.post.*`.
- Each item integration declares exactly one of `path`, `env`, `eval`, `source`, `aliases`, or
  `fragment`; complex fragments live in `profile/<item>.*`.
- Named `[profiles.*]` tables select items only. Successful items are activation-additive; explicit
  `sys profile disable` is the removal path.
- Do not add a platform-wide dispatcher or a parallel status/update protocol.
- External or overlay install scripts and executable profile code require
  `shine trust grant sys/<item>`;
  static detection/provider metadata and declarative PATH/env/aliases remain inspectable.

Verify with:

```bash
cargo build --target-dir target
cargo run --target-dir target -- sys list
cargo run --target-dir target -- sys info <item>
cargo run --target-dir target -- sys bootstrap <item> --dry-run
```

The sys commands may initialize config state. Use an isolated `SHINE_CONFIG_DIR` and copy the OS
preset into its `presets/sys/<os_id>/` tree when the check must not touch the real Shine directory.
