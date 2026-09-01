# 0041 — Preset permissions use versioned target-local declarations

- **Status**: Accepted
- **Date**: 2026-08-29
- **Evidence**: `core/src/permission.rs`, `core/src/runtime/validation.rs`, `presets/`

## Context

ADR 0040 established a fail-closed security Plan contract, but Presets had no common way to state
filesystem, network, command, administrator, environment/secret, or system capabilities. Existing
App destinations, Shell command metadata, Sys package providers, `requires_admin`,
`allow_app_hooks`, and `allow_sys_code` cover different concerns and cannot be treated as one
reviewable declaration. A category-wide permission union would also make a targeted Shell command
or Sys item inherit unrelated capabilities.

## Decision

Permission declaration schema v1 is owned by `shine-core`. App categories declare one top-level
`[permissions]` table, while every Shell `[[files]]` and Sys `[[items]]` entry declares its own
target-local permission table. All placements use `schema_version = 1` and grouped filesystem,
network, command, administrator, environment, and system fields.

Filesystem declarations use an access list plus a structured `home`, `shine`, `data-dir`,
`preset`, or `absolute` base. Preset paths are logical category-relative identities, never physical
checkout paths. Commands contain one program identity without argv; environment entries contain
only a variable name and `plain` or `secret` sensitivity; network entries contain a host identity
or explicit `any` scope. Unknown fields, unsupported versions, invalid identities, and duplicate
normalized permissions fail static validation.

Typed metadata remains the declaration for effects already bounded by Core, including managed App
destinations, Shell launcher/profile ownership, manifests and receipts, fixed package providers,
and managed Sys targets. The explicit permission table records additional or opaque capabilities;
static validation does not inspect script bodies or claim that an opaque script is complete.

Missing declarations remain a compatibility warning for external Presets during this delivery
slice. A declaration is descriptive, not a grant: it does not bypass `allow_app_hooks` or
`allow_sys_code`, and current lifecycle execution is unchanged. Future pure planners will combine
typed metadata and explicit declarations, mark uncomputable capabilities as blockers, and only a
later enforcement slice may make declarations mandatory for mutation.

## Consequences

- App, Shell, and Sys share one authoring vocabulary without collapsing their domain models.
- Command- and item-scoped operations do not inherit a category-wide permission union.
- `preset validate` can guide migration now without breaking existing external repositories.
- Built-in Presets and generated templates exercise the new schema with zero validation warnings.
- Coarse external-code grants remain compatibility gates until the separately planned trust
  migration.
