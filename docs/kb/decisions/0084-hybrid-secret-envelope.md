# 0084 — Versioned GPG / age data-key envelope

- **Status**: accepted for implementation; not independently audited
- **Scope**: workspace sealing and the shared secret reader

## Protocol

`hybrid:` followed by canonical padded RFC 4648 standard Base64 encodes this exact
binary sequence: version `01`, algorithm `01` (XChaCha20-Poly1305), 24-byte nonce,
three unsigned big-endian u32 lengths (GPG wrapper, age wrapper, encrypted data),
GPG wrapper bytes, age wrapper bytes, encrypted data including its 16-byte tag.
There are no optional fields, extensions, duplicate branches, or trailing bytes.
Unknown versions/algorithms and noncanonical encoding fail before tool invocation.
Each wrapper is nonempty and at most 1 MiB; plaintext is at most 16 MiB.
The entire header and both wrappers, in the above order, are AEAD associated data.
This authenticates all metadata and both access paths without self-reference.

Use RustCrypto `chacha20poly1305` 0.10.1 and `getrandom` 0.2 OS randomness. Each seal
uses one fresh 32-byte key and one fresh 24-byte nonce for exactly one encryption.
Never reuse a key, persist it, or pass it in argv. Zeroizing buffers own key material
and decrypted wrapper text, including error paths. External processes necessarily
have their own memory lifecycle; Rust cannot erase their memory or OS pipe buffers.
The wrapped plaintext is the exact ASCII string `shine-hybrid-key-v1:` followed by
canonical standard Base64 of nonce || key (56 bytes). Both length and nonce must
match before constructing the cipher. The textual adapter never loses binary bytes. Unwrap stdout is bounded to 96 bytes,
and captured diagnostics to 64 KiB; output overflow terminates the child. GPG unwrap
also uses `--no-options --output -` so local output-file options cannot persist a data key.
The reproducible vector in `cli/src/secret/hybrid-vector.txt` uses key byte `07` repeated
32 times, nonce byte `09` repeated 24 times, ASCII wrappers `gpg-wrapper` and `age-wrapper`,
and plaintext `secret` followed by LF. Protocol tests compare the complete envelope
and reject every single-byte mutation and truncation. These fixed wrappers are test
fixtures, not decryptable GPG/age packets. Real interoperability uses disposable keys.

## Tools and recipients

Hybrid requires GnuPG 2.2–2.5 and age 1.3 or newer. GPG recipients must be complete 40-hex
primary fingerprints, never names, groups, key IDs, or subkey selectors. Inspect
local public keys only using `--no-options` as the first argument, batch colon output,
`--no-auto-key-locate`, and `--no-auto-check-trustdb`. Reject absent, revoked, expired,
disabled, or ambiguous primary keys. Select a valid encryption subkey (or encryption
primary), freeze its full fingerprint and use the `!` exact-key suffix.
All hybrid GPG encryption ignores option files and explicitly disables encrypt-to,
default recipients and automatic retrieval; trust-model always is safe here because
identity was selected by its complete locally resolved fingerprint. GNUPGHOME still
selects the existing keyring; no import, download, or user configuration edit occurs.
The executable and keyring are trusted local dependencies, not adversarial programs.

age receives only explicit repeated `-r` arguments, no identity or recipients files.
Its standard CLI has no implicit recipient configuration. Recipient plugins remain
trusted local dependencies. Before old-payload decryption, both tools encrypt a
public fixed probe using the frozen lists to validate tool versions, recipient
availability, and plugin support. Probe success alone is not recipient isolation:
GPG option suppression and exact fingerprint selection enforce that boundary.
Failure in either real wrapping operation aborts without writing a partial envelope.
Tool errors are terminal; never infer retry or cancellation from stderr wording.

## Local selection and integration

The workspace preference and cache-bypass rules below describe the initial implementation;
[ADR 0085](0085-local-hybrid-preferences-and-cache.md) supersedes those two rules.
Broker authorization, wire routing and failure-without-fallback still apply.

`hybrid_decrypt_backend` is global-only (`gpg` or `age`). Project loading cannot
replace it and project saving cannot materialize it. Single-backend routing remains
unchanged. Hybrid parsing precedes candidate selection; explicit selection invokes
only that tool. Without a preference, probe command presence and age identity/plugin
availability without decrypting: one candidate is used, two require local TTY
selection, zero fail. Failure never falls back. Broker selection remains downstream
of local release authorization and cannot be provided by the remote request.

Runtime captures workspace and consumed source bytes once, hashes those bytes, and
compiles the same snapshots. Hybrid policy or a consumed `hybrid:` payload bypasses
both cache read and write, including malformed tagged payloads. Other modes do not
contribute. Existing source/workspace/payload schema versions stay unchanged.

## Sealing concurrency

Workspace and source-scoped exclusive advisory locks cover snapshot capture, preflight,
decryption, preparation, final byte comparisons and atomic replacement. File-only
sealing uses the same lock scheme scoped to its source. Concurrent cooperating Shine
sealers cannot interleave checks and replacements. Before each replacement, compare
both workspace and source complete bytes with their captured snapshots; disappearance
or inability to compare aborts, preserving the external state. Earlier completed
files stay completed and the error reports completed/current/unprocessed counts.

This is not a portable filesystem compare-and-swap against noncooperating editors.
An editor that ignores the lock and writes in the final comparison-to-rename interval
can still race; callers must exclude such writers during sealing. Hardware-wait edits
are detected at the final comparison. This boundary is explicit, rather than claiming
that a hash check followed by rename is universally race-free. Locks are released
by the OS on exit; lock files contain no secrets and must not be unlinked on release.

## References

- [RustCrypto implementation](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/)
- [GnuPG options](https://gnupg.org/documentation/manuals/gnupg/GPG-Configuration-Options.html)
- [Prior tagged routing](0008-age-secret-backend-tagged-ciphertext.md)


## Validation

The ignored `real_gpg_age_and_implicit_recipient_isolation` Rust test runs real tools
in disposable keyrings and confirms both access paths and exclusion of extra GPG
recipients. `scripts/test-hybrid-secrets.py` provides reproducible CLI fault injection,
source/policy change checkpoints, multi-file outcomes, matching stale-cache bypass,
and exact decrypt stdout. Hardware and non-macOS behavior remain unverified; see the
[implementation record](../../hybrid-secret-encryption-prd.md#13-实施记录2026-09-08).
