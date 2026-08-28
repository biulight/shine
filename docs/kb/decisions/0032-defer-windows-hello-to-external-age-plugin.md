# 0032 — Defer Windows Hello key custody to an external age plugin

- **Status**: accepted
- **Evidence**: Windows 11 23H2 with WebAuthn API v7; Windows 11 25H2 with WebAuthn API v9;
  [ADR 0008](0008-age-secret-backend-tagged-ciphertext.md)

## Context

Windows users need a way to require an independent user-verification gesture before an age
private-key operation, without leaving a reusable identity in an agent-readable file. A standalone
PoC tested whether Microsoft Passport KSP or the built-in Windows Hello WebAuthn authenticator could
protect a native age identity while preserving Shine's existing `age:` ciphertext and
multi-recipient behavior.

The same capability boundary was reproduced on two Windows 11 hosts:

| Primitive | Result | Consequence |
| --- | --- | --- |
| Passport RSA PKCS#1 v1.5 decrypt | Supported; fresh Hello prompts and cancellation fail closed | Compatibility-only construction |
| Passport RSA OAEP-SHA256 | `NTE_INVALID_PARAMETER` | No provider-managed modern RSA wrapping |
| Passport ECDH P-256 | Not supported | No standard ECDH-derived wrapping key |
| Passport raw RSA | `NTE_BAD_FLAGS` | No application-layer OAEP over Passport RSA |
| WebAuthn PRF / HMAC-secret | Credential returned `bPrfEnabled=false` | No local symmetric PRF wrapping key |

The working PKCS#1 v1.5 path also demonstrated non-exportable device binding, a separate Hello
prompt per unwrap, cancellation, cross-device failure, and interoperability with a ciphertext that
also carried a macOS Secure Enclave recipient. Those behavioral results do not make the rejected
key-transport construction suitable as a production default.

## Decision

- Shine will not add or retain a built-in Windows Hello/TPM age backend, including as an
  experimental feature.
- Platform or phone key custody belongs behind the standard age plugin protocol. A standalone
  `age-plugin-phone` project is a candidate adapter, not a Shine workspace member or dependency.
- Shine's `age:` ciphertext tag, recipient configuration, multi-recipient behavior, and external
  `age` process boundary remain unchanged.
- Failure or missing plugin support must never fall back to DPAPI, a desktop file identity, an
  ordinary password, TOTP seed material, or an authorization cache.
- The Windows Hello PoC source is removed after recording the capability results; one-off probe code
  is not a maintained product surface.

## Consequences

- This decision adds no user-visible Shine feature and requires no public-manual change.
- Windows users continue to use an already supported age identity or GPG until a compatible external
  plugin is independently ready.
- Re-evaluation requires a reviewed modern construction, fresh user verification for every private
  operation, non-exportable long-term key material, explicit recovery and device-replacement
  behavior, fail-closed cancellation and transport errors, and interoperability with native age
  multi-recipient ciphertext.
- Shine integration with a future plugin should be limited to ordinary age identity and recipient
  configuration; the plugin must not introduce Shine-specific ciphertext or expose long-term keys
  to Shine.
