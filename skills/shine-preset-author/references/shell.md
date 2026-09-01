# Shell preset authoring

Use a shell preset for commands that Shine exposes through its managed bin
directory. Start from `shine preset new shell`; its template is authoritative.

## Essential shape

- `description` explains the command group.
- Each `[[files]]` entry declares a safe relative `source` and optional plain
  filename `target` used as the command name.
- Native sources end in `.sh` or `.ps1`. `platforms` accepts exact `macos`, `linux`, and `windows`
  selectors; `unix` groups macOS and Linux. The array must not be empty, and overlapping exact/group
  entries for the same command are rejected on the affected OS.
- `needs_source = true` is for native scripts that must run in the caller's
  shell. It cannot be combined with `runtime = "bun"`.

Command names must be plain filenames and unique within every platform branch.
Keep all sources inside the category; do not use absolute paths or `..`.

Every `[[files]]` command has its own `[files.permissions]` table with
`schema_version = 1`. Platform variants of the same command declare separately.
Record program identities without argv and environment names without values;
classify every environment entry as `plain` or `secret`. Standard launcher,
snapshot, receipt, and profile ownership remains derived from Shell metadata.

## Bun and transforms

For cross-platform TypeScript or JavaScript helpers, set `runtime = "bun"` and
use a `.ts`, `.js`, `.mts`, or `.mjs` source. Bun entries may declare `env` and
`transforms`; native entries cannot declare runtime environment injection. The
source name in an `env = ["SOURCE=TARGET"]` alias is the permission identity.

If Bun dependencies are needed, include both `package.json` and `bun.lock` at
the category root. Do not declare `trustedDependencies`. Shine does not install
Bun, so report it as a prerequisite.

The isolated `shine shell install <name> --dry-run` resolves sources, Bun policy,
and intended links without creating files, links, manifests, or profile edits.
