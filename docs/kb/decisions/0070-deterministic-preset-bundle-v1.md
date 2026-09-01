# 0070 — Preset bundle v1 is a deterministic, unsigned tar.gz

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/runtime/pack.rs`, `cli/src/preset_pack.rs`

## Context

Roadmap Phase 5 requires reproducible packing before Phase 9 adds signing and registry distribution.
Packing the physical checkout directly would preserve machine paths, timestamps, ownership,
enumeration order, ignored dependency trees, and author-only fixtures. Reusing a generic archive
command would also bypass Preset validation and executable provenance policy.

## Decision

`shine preset pack` accepts exactly one validation-clean category and emits bundle schema v1 as a
gzip-compressed tar archive. Core builds the bytes; the CLI only performs the explicit atomic output
write. Entries are sorted by logical path. Tar uid, gid, and mtime are zero, regular modes normalize
to `0644` or `0755`, and the gzip mtime and operating-system byte are fixed. Checkout location and
filesystem enumeration order cannot affect bytes.

The first archive entry is `shine.bundle.json`, containing schema version, canonical category target,
and each packed category-relative path, normalized mode, and SHA-256. Category files follow below
`preset/<kind>/<name>/`. Author-only `shine.test.toml` is excluded.

Packing scans the physical category as well as the immutable snapshot. It rejects any
`node_modules`, symlink, secret-key filename or private-key material, private machine HOME path, and
executable/shebang file not referenced by metadata. Diagnostics are stable codes and never echo the
suspected secret or private path. Output inside the source category is rejected; replacing another
output requires explicit `--force`.

Bundle v1 is unsigned. Signature, author identity, revocation, registry upload, and remote dependency
resolution remain Phase 9 work and must wrap these exact deterministic bytes rather than changing
their meaning silently.

## Consequences

- Identical logical input produces byte-identical archives across checkout roots.
- Ignored files cannot evade pack policy merely because runtime snapshot traversal excludes them.
- Fixture evolution does not change distributable capability bytes.
- Bundle format or policy reinterpretation requires a new schema/ADR; additive report fields do not.
