# 0083 — Secret Base64 encoding runs in process

- **Status**: accepted
- **Evidence**: `cli/src/secret/{exec,gpg,age,mod}.rs`

## Context

Secret adapters invoked an external `base64` command and retried platform-specific decode flags.
The CLI already depends on the Rust `base64` crate for SSH. Encoding ciphertext does not require
an external crypto or hardware integration, so this extra runtime dependency adds avoidable failure
modes.

## Decision

- Use the existing Rust crate for standard padded, single-line Base64 encoding and decoding in
  both secret backends. Remove only the external Base64 dependency; keep GPG, age, and identity
  plugins external, including their interaction and authorization behavior.
- Strip ASCII whitespace before decoding to accept wrapped ciphertext. Reject invalid alphabet,
  missing/incorrect padding, and nonzero unused trailing bits; do not reproduce platform-specific
  acceptance or silent truncation of malformed input. Decode fully before writing ciphertext.
- Preserve untagged GPG ciphertext, the `age:` prefix, and the private temporary-file lifecycle.
- Add no public Base64 command. The Unix `copyfile` preset retains its own external dependencies.

## Consequences

Existing Shine-generated ciphertext remains compatible without migration. Malformed or nonstandard
inputs previously accepted by some host utilities now fail consistently. Backend-adapter tests use
controlled tools on an isolated PATH without `base64`; codec tests run without external tools.
This refines the external-process implementation described in ADR 0008, without changing its
backend selection or cryptographic design.
