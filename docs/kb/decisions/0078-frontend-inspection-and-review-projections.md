# 0078 — Frontend inspection and review preserve domain semantics

- **Status**: accepted
- **Scope**: Roadmap Phase 6B

## Decision

`FrontendService` projects the existing domain inspection into a versioned report of canonical
targets, opaque resource identities, typed states, applicable operations, and safe diagnostics.
An applicable operation is guidance, never authorization or a promise that its security Plan is
ready. Manual generator differences require refresh, not upgrade. Unevaluated generators remain
explicitly unknown. A Sys bootstrap receipt means recorded, not that third-party software is current.

Trusted local frontends may receive non-serializable inspection details (paths, contents, local
errors, and domain presentation metadata) alongside the safe report from the same observation.
They must not perform a second inspection to render a diff. Ordinary inspection does not execute
Preset code. Explicit generator evaluation remains a separate trusted-local option and is never
part of Plan review or the ordinary read-only adapter surface.

The service accepts a workspace-local typed review request covering all existing security Plan
operations, including recovery and specialized App/Sys operations. It returns a versioned wrapper
around the existing `PlanV1`; the request stays local and is retained without losing input versions.
No review report or request contains approval authority. Trusted human frontends continue to own
confirmation; Core still regenerates and matches the exact Plan before execution.

## Compatibility and verification

CLI output, explicit generator evaluation, exit behavior, and local diffs remain compatible.
Fixtures cover the three domains, missing sources, conflicts, manual refresh, safe serialization,
and service/Core Plan equivalence. Adapter code may format and group reports but must not derive
ownership, operation applicability, or lifecycle outcomes independently.

This adds workspace service contracts, not public CLI JSON commands or a general Rust API.
