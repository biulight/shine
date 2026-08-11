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
private key never leaves the enclave, and its public recipient (`age1se1...`) can be encrypted to
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
- `shine env secret identity init [--touch-id]` generates an age identity (`age-keygen` or
  `age-plugin-se keygen`) and prints its recipient; the macOS requirement for `--touch-id` is
  checked at **runtime** (`std::env::consts::OS`), not compile time, since plain age identities
  work on every OS and the rest of the CLI is not platform-gated at compile time either.
- Recipient/backend precedence for `encrypt`/`seal`: CLI flag > workspace `env.encryption` >
  `config.toml` (`gpg_recipients`/`age_recipients`/`secret_backend`) > default (GPG). Resolution
  helpers return `Option`, not `Result`, when used for `seal`, so sealing a file with no `[secret]`
  entries never requires a recipient to be configured.

## Consequences

- Existing GPG secrets keep decrypting unmodified — no migration is required.
- Rotating an age identity or dropping a recipient from `age_recipients` does **not** rotate
  secrets already committed to history; re-`seal`ing re-encrypts to the current recipient list,
  but old ciphertext (e.g. in git history) remains decryptable by the identity it was originally
  sealed for.
- `age`/`age-plugin-se` are external dependencies (like `gpg`) the user must install; shine detects
  their absence with a clear preflight error rather than a raw subprocess failure.
