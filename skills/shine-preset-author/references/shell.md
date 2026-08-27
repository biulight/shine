# Shell preset authoring

Use a shell preset for commands that Shine exposes through its managed bin
directory. Start from `shine preset new shell`; its template is authoritative.

## Essential shape

- `description` explains the command group.
- Each `[[files]]` entry declares a safe relative `source` and optional plain
  filename `target` used as the command name.
- Native sources end in `.sh` or `.ps1`. Use `platforms` to make parallel Unix
  and Windows implementations of the same command mutually exclusive.
- `needs_source = true` is for native scripts that must run in the caller's
  shell. It cannot be combined with `runtime = "bun"`.

Command names must be plain filenames and unique within every platform branch.
Keep all sources inside the category; do not use absolute paths or `..`.

## Bun and transforms

For cross-platform TypeScript or JavaScript helpers, set `runtime = "bun"` and
use a `.ts`, `.js`, `.mts`, or `.mjs` source. Bun entries may declare `env` and
`transforms`; native entries cannot declare runtime environment injection.

If Bun dependencies are needed, include both `package.json` and `bun.lock` at
the category root. Do not declare `trustedDependencies`. Shine does not install
Bun, so report it as a prerequisite.

The isolated `shine shell install <name> --dry-run` resolves sources, Bun policy,
and intended links without creating files, links, manifests, or profile edits.
