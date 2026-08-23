# 0031 — External Bun presets may use locked runtime dependencies

- **Status**: accepted
- **Evidence**: `cli/src/bun_runtime.rs`, `cli/src/bin_links.rs`, `cli/src/apps/build.rs`,
  `cli/src/apps/generator.rs`

## Context

Bun is used by Shell commands, app artifacts and app generators. Requiring every external preset
to bundle all JavaScript would make ordinary package reuse awkward, while allowing bare imports to
install implicitly would make built-in presets non-reproducible and turn read-oriented Shine
operations into hidden network activity. Having Shine install packages itself would also require a
new package lifecycle, cache ownership model and uninstall policy that duplicate Bun.

Overlay resolution adds another boundary: package metadata beside an overlay must authorize only a
script physically supplied by that overlay. It must not change how an inherited embedded script is
executed.

## Decision

Built-in Bun scripts are self-contained. They may import relative modules and Bun/Node built-ins,
and always run with `bun --no-install`.

An external or overlay Bun script may use registry packages only when `package.json` and `bun.lock`
both exist in the physical category directory that owns the effective script. That script runs with
`bun --install=fallback`; a missing package/lock counterpart is an error. The first version rejects
the `trustedDependencies` field and makes no compatibility promise for native extensions,
workspaces, `file:`, `link:`, or dependencies requiring lifecycle scripts.

Shine does not run `bun install`, create or remove `node_modules`, or own Bun's global cache and
virtual store. Snapshot and overlay traversal ignore every `node_modules/` directory while retaining
the package manifest and lock. Package download can therefore happen only when an eligible script
actually executes, subject to the existing external-code permission gates.

Shell manifests record the dependency mode and a content hash of `package.json` plus `bun.lock`.
Snapshot lock changes require upgrade; live commands consume the current files on their next run
while status still reports the receipt change. App artifact, teardown and generator runners resolve
the same policy from the final script source. Embedded temporary generator scripts always use
`--no-install`.

## Consequences

- Unlocked external Bun scripts remain usable, but bare package imports fail instead of downloading.
- A category's first dependency-using execution may require network access; offline operation needs
  Bun's cache to be warm or the preset author to bundle/vendor dependencies.
- Shine uninstall never cleans Bun caches, because those caches may be shared by unrelated tools.
- Future Python/uv or Deno support can reuse the same ownership boundary without sharing Bun-specific
  file conventions.
