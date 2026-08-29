# App preset authoring

Use an app preset for files installed into application configuration locations.
Start from `shine preset new app`; the emitted template is authoritative for the
installed Shine release.

## Essential shape

- `description` explains the category.
- `dest` is an absolute path after `~`/environment expansion, or a platform map with exact
  `macos`, `linux`, and `windows` values plus the optional `unix` macOS/Linux fallback. An exact
  destination wins over `unix`.
- Each `[[files]]` entry declares a safe relative `source`; `target` defaults to
  the source path. A per-file `dest` may override the category destination.
- `platforms` accepts `macos`, `linux`, `windows`, and the macOS/Linux compatibility group `unix`.
  The array must not be empty. Validate all three exact OS branches on every host.

Prefer explicit file lists. Keep sources and generator/artifact scripts inside
the category. Never use absolute source paths or `..`.

## Permission declaration

Every App category has one top-level `[permissions]` table with
`schema_version = 1`. Ordinary managed destinations and receipt operations are
already bounded by typed App metadata; use the table for additional commands,
network access, environment names, administrator authorization, system
capabilities, or filesystem effects of hooks, generators, and artifacts.

Environment entries contain only a name and `plain`/`secret` sensitivity.
Filesystem entries use `access`, a structured `base` (`home`, `shine`,
`data-dir`, `preset`, or `absolute`), and a normalized path. A declaration does
not enable external code: `allow_app_hooks` remains a separate user opt-in.

## Optional behavior

- `transforms` supports only transforms accepted by the current validator,
  commonly `template` and `jsonc-to-json`.
- `install_mode = "json-merge"` requires non-empty, top-level `managed_keys`.
- A generator declares `script`, optional `runtime = "bun"`, `env`, and a
  `when_env` key included in `env`. Always provide a static source fallback.
- `post_install` and `post_upgrade` hooks are argv declarations. Validation does
  not run them, and this skill must never run them.
- `[artifact]` may declare `script`, optional `teardown`, `runtime`, and an
  explicit `env` allowlist. Every environment source must also have a
  sensitivity entry in the category permission declaration. This skill
  validates referenced files but never applies or removes an artifact. A hook
  that deliberately invokes artifact apply must pass `--yes` for its
  non-interactive child Plan.
- Bun code uses `.ts`, `.js`, `.mts`, or `.mjs`. If dependencies are needed,
  place both `package.json` and `bun.lock` at the category root; never declare
  `trustedDependencies`.

Avoid duplicate effective destinations on any operating system. Keep destination
ownership narrow and explain required secrets or environment keys without
placing their values in the preset.
