# 0075 — Phone identity setup uses a public plugin handoff

- **Status**: accepted
- **Evidence**: `cli/src/env/identity.rs`, `cli/src/config/{mod,load}.rs`,
  `age-plugin-phone setup --json`

## Context

`shine env secret identity init --touch-id` can choose Shine's default identity path because
`age-plugin-se keygen` creates one local identity file. Phone custody also owns pairing, transport,
TPM keys, replay state, a private locator, interruption recovery, and a randomly allocated public
stub. Reproducing or discovering those paths in Shine would split the plugin's lifecycle and safety
boundary. Parsing its human terminal output would also make integration depend on presentation.

Users may already have a normal or Secure Enclave identity. Replacing it when phone setup succeeds
would silently remove an existing decryption route, while copying plugin stubs into a Shine-owned
file would leave stale aliases after plugin cleanup.

## Decision

- `shine env secret identity init --phone` invokes the standalone plugin's transactional `setup`
  command and consumes only its versioned public JSON result: the identity-stub path and recipient.
- Shine explicitly requests `--recipient-type tag` by default and supports opt-in `phone`.
  This does not change the standalone plugin default. Tag setup preflights age 1.3 before pairing;
  both plugin and phone app must support `p256tag`. Unsupported setup never retries or falls back.
  Schema v1 is unchanged; the returned type, Bech32 encoding, tag key structure, and stub comment
  must agree before global configuration is saved.
- Existing phone recipients are not SE-style HRP migrations: v1/v2 have different payloads and
  wrapping. Users export tag via the plugin's read-only `recipients -i ... --recipient-type tag`,
  then explicitly replace configuration and reseal after upgrading both ends. Tagged recipients
  accept public target discoverability in exchange for native encryption.
- Pairing interaction remains on the inherited terminal. Shine never selects plugin private paths,
  reads private state, performs cleanup, or treats configuration failure as permission to revoke a
  pairing.
- Global configuration retains the legacy `age_identity` string and adds `age_identities` as an
  ordered list of additional paths. Runtime resolution merges and deduplicates both forms. An
  existing implicit default identity is made explicit before a phone stub is appended.
- A project that explicitly supplies either identity setting replaces the complete global identity
  set. Phone setup refuses before pairing when such a project override is active.
- The command changes neither `secret_backend` nor `age_recipients`. The phone recipient must be
  paired with an independently verified recovery recipient by an explicit user configuration.

## Consequences

- Phone setup remains usable by any standard age client and has no Shine-specific ciphertext or
  runtime protocol.
- Multiple local identities can coexist without copying private identities or public plugin stubs.
- If pairing succeeds but the atomic global-config write fails, the pairing remains active and the
  CLI prints a TOML-safe manual configuration. Starting another setup is never suggested as repair.
- The phone shortcut remains limited to the plugin's Windows Alpha support and synthetic-data
  posture; stable use still requires the plugin's independent lifecycle and recovery validation.
