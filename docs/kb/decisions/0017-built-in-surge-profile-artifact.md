# 0017 — Surge profile include management is a built-in Bun artifact

- **Status**: accepted
- **Evidence**: `presets/app/surge/profile-artifact.ts`,
  `presets/app/surge/build.ts`, `presets/app/surge/unbuild.ts`,
  `presets/app/surge/profile-artifact.test.ts`
- **Supersedes**: ADR 0009/0012's placement of the canonical Surge artifact in
  a private overlay. Their explicit-build and symmetric-teardown semantics
  remain unchanged.

## Context

The original Surge artifact lived in `shineOverlay` because profile patching was
assumed to contain provider-specific behavior. In practice the script only
maintained three stable mappings:

- `[Proxy]` → `local-proxies.conf`, appended;
- `[Proxy Group]` → `local-proxy-groups.conf`, appended;
- `[Rule]` → `local-rules.conf`, prepended for rule priority.

The provider URL and profile contents are opaque operands. User-specific policy
definitions still belong in the overlay, but the patch algorithm does not.
Keeping the algorithm private also left users without that overlay on a no-op
built-in placeholder, and the overlay had no matching teardown implementation.

The shell/awk implementation had additional safety gaps: a missing
`#!include` silently succeeded, replacement forced mode `0644`, CRLF handling
was fragile, and replacing a symlink path could destroy the symlink.

## Decision

Ship the Surge artifact in the embedded preset as Bun TypeScript:

- `[artifact]` uses `build.ts`, `unbuild.ts`, and `runtime = "bun"`.
- Shared parsing and filesystem behavior live in `profile-artifact.ts`.
- `SURGE_PROFILE` selects the user-owned profile. Overlay policy files remain
  ordinary installed preset content; an overlay only replaces the artifact if
  it intentionally supplies the exact `build.ts` or `unbuild.ts` path.
- Build and unbuild remain explicit. Install and upgrade never patch the active
  profile implicitly; uninstall retains ADR 0012's best-effort teardown.
- Rewrites are idempotent and same-directory atomic, preserve the profile's mode
  and exact per-line endings, reject symlink profiles and invalid UTF-8, and
  never print profile contents or provider include operands.
- Build fails if a local file exists but its corresponding section has no
  patchable `#!include`, avoiding a false-success state.
- Unbuild removes all matching local operands and removes a directive that
  would otherwise become empty.

## Consequences

- `shineOverlay/app/surge/build.sh` is redundant and can be removed after users
  upgrade to a Shine version containing this preset.
- Both explicit Surge artifact commands require Bun even when the URI
  subscription generator is disabled. This is acceptable because they are
  opt-in commands, Bun is already the preset runtime for subscription
  conversion, and the failure path clearly reports a missing runtime.
- The generic Rust artifact runner remains unchanged and contains no
  Surge-specific syntax.
- Profile parsing is covered by Bun tests for precedence, idempotency, CRLF,
  permissions, missing includes, teardown, and symlink rejection.
