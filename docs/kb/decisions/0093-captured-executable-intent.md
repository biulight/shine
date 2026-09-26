# 0093 — Capture executable intent with Preset bytes

- **Status**: Accepted
- **Date**: 2026-09-26
- **Evidence**: `core/src/runtime/{preset,bootstrap,validation,app_script}.rs`
- **Extends**: ADR 0091, ADR 0092

## Context

Private App execution copies retained source bytes but lost executable helper permissions. A native
entrypoint calling an extensionless helper worked in its checkout and failed with exit 126 from
the captured copy. Restoring permissions by reopening the source during execution would break the
snapshot boundary.

## Decision

External and overlay capture records a boolean executable flag from Unix mode bits `0o111` beside
each file's bytes. Capture through the authoring/validation path uses the same rule. Only effective
files contribute to Plan and code digests; a shadowed base flag has no effect. An executable flag
adds a framed marker to that file's digest input. Non-executable files keep their earlier framing.
Checkout paths, ownership, other permission bits, and setuid/setgid flags remain excluded.

App execution sets executable bits on captured helpers in both the effective source copy and the
overlay subset. Native entrypoints retain their existing explicit executable treatment. Embedded
distribution inputs remain byte-only unless their builder supplies executable intent explicitly.

## Consequences

- Changing executable intent invalidates a reviewed Plan and matching snapshot grants, even when
  file bytes are unchanged. Existing grants containing executable external files need review again.
- Development grants retain their source-scoped semantics, but every operation still requires a
  Plan matching the freshly captured executable intent.
- Permission edits after capture cannot change that invocation. Source ownership and privileged
  mode bits are never copied into execution trees. Windows capture has no Unix executable flag.
