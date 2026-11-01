# App preset authoring

Use an app preset for files installed into application configuration locations.
Follow the skill's isolated scaffolding workflow; use the installed template and validator
for the supported metadata version (ordinary App metadata uses version 2; managed secret files require version 3).

## Essential shape

- `description` explains the category.
- `dest` is an absolute path after `~`/environment expansion, or a platform map with exact
  `macos`, `linux`, and `windows` values plus the optional `unix` macOS/Linux fallback. An exact
  destination wins over `unix`.
- Each `[[files]]` entry declares a safe relative `source`; `target` defaults to
  the source path. A per-file `dest` may override the category destination.
- `platforms` accepts `macos`, `linux`, `windows`, and the macOS/Linux compatibility group `unix`.
  The array must not be empty. Static validation checks all three exact OS branches on every host.

Prefer explicit file lists. Keep sources and generator/artifact scripts inside
the category. Never use absolute source paths or `..`.

## Capability statements and input contracts

An App category may have one top-level `[permissions]` table. Do not create an
empty table or repeat hook commands, runtime commands, script paths, managed
destinations, or receipt operations already expressed by typed metadata. Declare statically
identified effects of App Bun scripts: definite declaration differences and parse failures block
execution. Additional command/filesystem/network/system entries are review context. Environment
names and Administrator authorization remain required when Shine injects or elevates them.

Additional `[permissions].environment` entries contain only a name and `plain`/`secret` sensitivity.
Filesystem entries use `access`, a structured `base` (`home`, `shine`,
`data-dir`, `preset`, or `absolute`), and a normalized path. A statement does not
enable, restrict, or sandbox code: external/overlay executable entries require separate
target-scoped trust for the complete effective category snapshot.

## Optional behavior

- `transforms` supports only transforms accepted by the current validator,
  commonly `template` and `jsonc-to-json`.
- `install_mode = "json-merge"` requires non-empty, top-level `managed_keys`.
- A generator declares `script`, optional `runtime = "bun"`, `env`, and a
  `when_env` key included in `env`. Always provide a static source fallback.
- `post_install` and `post_upgrade` hooks declare exactly one of `command` (direct argv)
  or `script` (optional `runtime = "bun"`). Script hooks share the parent lifecycle Plan;
  declare required environment inputs there; Core derives script/runtime execution identities.
  Never launch `shine app artifact apply` from a hook or add `--yes` to compose nested approvals.
- `[artifact]` may declare `script`, optional `teardown`, `runtime`, and an
  explicit structured `env` allowlist: `source`, optional `target`, and required `sensitivity`.
  Core derives category environment permissions from these inputs; no duplicate
  `[permissions].environment` declaration is needed. Artifact apply/remove remains
  an explicit operation with its own reviewed Plan, outside this authoring workflow.
- Bun code uses `.ts`, `.js`, `.mts`, or `.mjs`. If dependencies are needed,
  place both `package.json` and `bun.lock` at the category root; never declare
  `trustedDependencies`.

Avoid duplicate effective destinations on any operating system. Keep destination
ownership narrow and explain required secrets or environment keys without
placing their values in the preset.

## Static code checks

Run lint for every App entry, including generators not triggered by current inputs. Supported local
JavaScript/TypeScript dependencies are analyzed without downloading packages or executing code.
Fix definite differences at the reported source position by correcting the declaration or script.
Dynamic targets, unsupported languages/packages and subprocess effects remain explicit coverage
gaps; neither trust nor unrestricted metadata removes them. Missing corroboration is advisory.
For structured `env`, declare `source`, optional `target`, and `sensitivity` together; permissions use the source identity. Fixed-variable
exemptions use exact documented keys, not a general `SHINE_` prefix. Do not use real values to make
analysis or synthetic planning succeed. Positions use one-based lines and UTF-8 byte columns.

## Managed secret files

Use metadata version 3 and `[[secret_files]]` with a category-relative `workspace`,
explicit `file_group`, and `mode`. Workspace v3 owns the source/target mappings.
`auto_decrypt` defaults to false. Workspace and ciphertext are managed auxiliary files;
do not also list them in `[[files]]`. Keep mapped plaintext targets out of the source
checkout, including ignored files: effective snapshots containing them are rejected.
Secret resources cannot use transforms, generators or JSON merge. Unix ownership and
private mode (`0400` or `0600`) are reviewed; Windows rejects Unix permission overrides.
Static validation and dry-runs never decrypt. Actual local/broker authorization and
service activation remain deployment steps.
