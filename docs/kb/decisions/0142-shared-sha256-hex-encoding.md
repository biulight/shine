# ADR 0142: Shared SHA-256 lowercase hexadecimal encoding

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Core's `digest.rs` owns only hashing exact raw bytes to lowercase hexadecimal and encoding an
already-computed 32-byte SHA-256 digest. It adds no prefix, namespace, framing, normalization,
validation, persistence or execution authority. One encoder preserves all leading zeroes and
always emits 64 lowercase hexadecimal characters. No dependency or hash algorithm changes.

Plan/snapshot construction retains its domain separators, sorted observations, big-endian length
frames, JSON serialization, schema versions, digest newtypes and strict decoders. Workspace input
hashing retains its existing incremental updates, little-endian cache version, missing-source
marker, path spelling and `sha256:` prefix; local-cache context retains JSON tuple encoding.
Broker policy, file-lock identities, maintenance scopes, frontend report IDs and preset package
or migration hashes retain their own input organization and validation. Existing public `sha256`
exports remain available through aliases. Content receipt hashes remain FNV and are not migrated.

Only the final hash/hex primitives are shared. This is not a common fingerprint protocol or an
opportunity to change an existing protocol's framing; such changes require independent versioning.
Approval, locks, receipts, journals and recovery dispatch remain unchanged.

## Validation

Before extraction, capture fixed snapshot, Plan fingerprint and workspace-input vectors from
`c431aeb`; keep exact expected values as permanent consumer regressions. Independently calculate
the workspace-input and local-cache JSON tuple vectors. Check standard SHA-256 known answers,
leading zeroes, every byte value and padding/block boundaries against both old hex encoders.
Keep invalid length, non-hex and uppercase digest rejection with the Plan owners. Existing Plan,
broker, cache, migration, package and recovery tests validate the consumers of the shared helper.
Test-only historical encoders remain independent compatibility oracles.

A second review used a temporary differential harness with the original `c431aeb` Plan module
and extracted original/current domain functions. All 708 output comparisons matched, covering
snapshot and Plan fingerprints, Plan JSON and approvals, workspace inputs and cache contexts,
package/migration hashes, broker inputs, lifecycle hashes, resource/recovery IDs, normalized
file-lock keys, project-cache paths and maintenance scopes. Cases included empty/binary inputs,
SHA-256 block boundaries, missing versus empty sources, source/recipient order, Unicode and
non-UTF-8 paths. Source review also confirmed unchanged domain versions, prefixes and framing.

Local macOS validation passed 1711 all-feature workspace tests (two skipped), including all four
new tests, and all-target/all-feature Clippy with warnings denied. Formatting, spelling and KB
relative-link checks passed. No native Windows/Linux execution was performed for this change.
