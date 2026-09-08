# 0008 — age is a second secret backend, routed by a ciphertext tag

- **Status**: accepted
- **Evidence**: `cli/src/secret/{mod,age,gpg,exec}.rs`, `cli/src/env/identity.rs`,
  `shine env secret encrypt/decrypt/seal/identity`

## Context

`shine env secret encrypt`/`decrypt` only supported GPG (YubiKey worked implicitly through
`gpg-agent`). Apple Touch ID support was requested, with a hard requirement: ciphertext must be
committable to a shared repo and decryptable by every teammate, not just the device that sealed
it. That rules out device-local designs (e.g. a Keychain item reference) and requires
multi-recipient encryption — the same ciphertext encrypted to every team member's public key.

`age` plus `age-plugin-se` fits this: `age-plugin-se` mints a Secure Enclave identity whose
private key never leaves the enclave, and its native tagged recipient (`age1tag1...`) can be encrypted to
alongside ordinary `age-keygen` recipients (`age1...`) for teammates without Touch ID. A CLI
binary cannot use biometry-gated Keychain APIs without Apple entitlements, so shelling out to
`age`/`age-plugin-se` — mirroring the existing `gpg`/`base64` external-process pattern in
`secret/gpg.rs` — was the strongest available design. No new Rust crypto dependency was added;
`secret/exec.rs` now holds the process-spawning helpers shared by both backends.

## Decision

- Ciphertext carries a backend tag: `age:<base64>` for age, **untagged base64 for GPG**
  (unchanged from before this backend existed). `secret::parse_tagged_ciphertext` is the single
  place that inspects the tag.
- **Decryption is purely tag-based** and never consults `Config::secret_backend`. Changing the
  default encrypt backend can therefore never break a secret encrypted before the change.
- `secret::encrypt_secret`/`decrypt_secret` are the only entry points call sites use; the
  previously-reserved `SecretBackend` trait and unused `GpgBackend` struct were removed since the
  final shape (recipient lists resolved per call, identities threaded into decrypt) didn't fit a
  per-instance trait object cleanly.
- GPG encryption now also accepts a recipient list (`gpg -r` repeated), matching age's
  multi-recipient shape, so `-r/--recipient` behaves the same way regardless of backend.
- `shine env secret identity init [--touch-id]` generates a local age identity (`age-keygen` or
  `age-plugin-se keygen --recipient-type=tag`) and prints its recipient; the macOS requirement for `--touch-id` is
  checked at **runtime** (`std::env::consts::OS`), not compile time, since plain age identities
  work on every OS and the rest of the CLI is not platform-gated at compile time either. The later
  `--phone` setup handoff is governed separately by [ADR 0075](0075-phone-identity-setup-handoff.md)
  and does not change this backend or ciphertext decision.
- Shine requires age 1.3 or newer and uses its native `age1tag` wrapping for new Secure Enclave
  recipients. Encryption therefore does not need the platform-specific plugin; decryption still
  does. `state migrate` converts the Bech32 HRP and checksum of configured legacy `age1se`
  recipients without changing their public-key payload or existing ciphertext. This deliberately
  accepts tagged-recipient target discoverability in exchange for cross-platform sealing.
- Recipient/backend precedence for `encrypt`/`seal`: CLI flag > workspace `env.encryption` >
  `config.toml` (`gpg_recipients`/`age_recipients`/`secret_backend`) > default (GPG). Resolution
  helpers return `Option`, not `Result`, when used for `seal`, so sealing a file with no `[secret]`
  entries never requires a recipient to be configured.

## Consequences

- [ADR 0084](0084-hybrid-secret-envelope.md) adds an explicitly selected workspace hybrid
  envelope. Its local-only preference chooses one listed key wrapper, while these original
  GPG/age ciphertext routes and encryption formats remain unchanged.

- [ADR 0083](0083-in-process-secret-base64.md) replaces the original external Base64 steps with
  in-process encoding; GPG, age, and identity plugins remain external.

- Existing GPG secrets keep decrypting unmodified — no migration is required.
- Rotating an age identity or dropping a recipient from `age_recipients` does **not** rotate
  secrets already committed to history; re-`seal`ing re-encrypts to the current recipient list,
  but old ciphertext (e.g. in git history) remains decryptable by the identity it was originally
  sealed for.
- age 1.3 or newer is required for every age operation. `age-plugin-se` is required only to generate
  and decrypt Secure Enclave identities; legacy `age1se` encryption remains compatible when the
  plugin is installed and otherwise reports the explicit migration path.
