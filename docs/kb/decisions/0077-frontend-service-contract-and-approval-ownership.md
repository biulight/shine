# 0077 — Frontends share versioned service contracts and human-owned approval

- **Status**: accepted
- **Evidence**: `core/src/frontend.rs`, `cli/src/{core_runtime,list}.rs`,
  `docs/frontend-service-conformance-prd.md`

## Context

`CoreRuntime`, `PlanV1`, `LifecycleResultV1`, host ports, and immutable Preset bootstrap already
centralize lifecycle behavior, but real-host inventory and presentation remain assembled by CLI
modules. Copying that assembly into MCP or UI adapters would create competing meanings for
installation, diagnostics, Plan review, recovery, and approval. Serializing current inspection
models or `RuntimeEvent` would also expose paths, content, child output, and raw local errors.

## Decision

`shine-core` owns a workspace-facing `FrontendService` over one captured `CoreRuntime`. Its stable
reports use explicit `V1` schemas and contain only canonical identities, typed safe states, and
stable diagnostic codes. Private paths, content, argv, environment values, secret plaintext, raw
errors, and process output never enter these reports. Local failures retain their source only in a
non-serializable error wrapper. Contract changes may add optional fields compatibly; changing field
meaning, identity, ordering, severity, or redaction requires a new schema version.

Inventory v1 is the first slice. It returns the deterministic union of effective-snapshot and
Shine-installed targets. `available` means present and platform-valid in the immutable snapshot;
`installed` means manifest/receipt evidence or target-launcher presence, not external software
detection; launcher ownership/conflict detail remains an inspection concern. Installed targets
without current Preset material receive a safe diagnostic. The CLI remains a rendering adapter and
preserves its released filtering and output.

`RuntimeEvent` remains non-serializable and may continue to carry CLI-private presentation data.
Future stable events require an explicit redacted projection. `CoreRuntime` remains doc-hidden and
workspace-internal; adding the service does not make its host or domain APIs a general third-party
Rust interface.

A Plan review request is not approval. Only a trusted human-facing CLI or UI may construct the
one-shot `PlanApprovalV1`, and only after an affirmative action over a freshly rendered ready Plan.
AI/MCP adapters may produce review requests but must not expose approval construction, accept model
claims of approval, pass equivalent `--yes` authority, or invoke mutation. Every apply still
recaptures inputs, regenerates the Plan, and validates its exact fingerprint and permissions.

## Consequences

- CLI, MCP, and UI gain one semantic source for stable inventory and later frontend contracts.
- Adapters may group and format contract values but cannot parse manifests or derive lifecycle,
  ownership, approval, or recovery decisions.
- Safe reports intentionally contain less detail than trusted local terminal errors and events.
- Slice 6A adds no public command or JSON output, so public manuals and changelog remain unchanged.
