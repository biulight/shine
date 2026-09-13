# 0085 — Personal workspace hybrid preference and local encrypted cache

- **Status**: accepted
- **Supersedes**: ADR 0084 global-only workspace selection and compiled-cache bypass

## Decision

Local workspace run, seal, and export read `shine.config.local.toml` beside the
selected workspace once. Only `hybrid_decrypt_backend` is accepted; absence inherits
the global preference, while invalid content fails. Shared `shine.config.toml`
continues to exclude this field. The personal file is ignored by Git and is trusted
as local project input, without a new approval prompt. Git ignore is not a security
boundary: a process able to edit this file can change the selected authorization
path. Programs with access to global user state were never isolated by this setting.
Broker snapshots neither carry nor load personal overrides; broker release approval
and the decrypting machine's configuration remain authoritative.

Hybrid-involving local `env run` validates captured source structure before selecting
one backend, then freezes that selection for the run. It caches compiled values
using that backend and only the corresponding workspace recipients. Missing recipient
lists skip caching, never inherit global recipients. GPG cache encryption preserves
full-fingerprint resolution, exact encryption-key selection and suppression of
implicit recipients; GPG cache decryption suppresses output-file options.

## Cache boundary

Separate per-mode GPG/age cache filenames exclude legacy compiled caches. Version,
input snapshot hash, selected backend and recipient list form a length-delimited
context hash. The hash is also inside the encrypted payload, so editing only outer
metadata cannot relabel an old cache. Format/context/backend misses compile the
captured sources; after cache decryption starts, failure/cancellation is terminal.
Encrypted payload parse errors omit plaintext diagnostics. Cache writes are private
and atomic; failure warns but does not block successfully compiled execution.

No TTL, daemon, persisted plaintext, data key, or authorization cache is introduced.
A hit still invokes the selected backend once. Backend changes select another cache;
workspace/source/recipient changes invalidate it. Old encrypted caches are not revoked
or deleted automatically. Source envelope structure is checked before cache reads;
AEAD integrity is verified when sources are compiled. Content changes cannot reuse a
previous cache by merely editing its outer input hash. As with source encryption,
public-key encryption does not authenticate the author of a whole replacement cache.

## Validation

Unit tests cover sparse local preferences, invalid configuration, caller isolation,
encrypted context binding and pre-decrypt backend checks. The isolated CLI regression
script covers single-tool readers, multi-source cache hits, preference switching,
input invalidation, malformed sources and terminal cancellation. Existing broker
suites retain the remote release boundary. Hardware behavior needs device validation.
