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

## Coverage

Keep fixed snapshot, Plan fingerprint and workspace-input vectors captured before extraction from
`c431aeb`, independently calculated cache vectors, standard SHA-256 known answers, leading zeroes
and every byte value. Plan owners retain invalid length, non-hex and uppercase rejection. Existing
broker, cache, migration, package and recovery tests exercise their own framing and consumers.

Historical encoders and block-boundary comparisons served the extraction audit; they are not
permanent parallel implementations. The shared encoder retains direct known-answer coverage.
Native platform behavior requires separate evidence from local compatibility tests.
