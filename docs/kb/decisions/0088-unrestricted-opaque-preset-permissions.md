# 0088 — Presets may declare unrestricted opaque-code effects

- **Status**: Accepted
- **Date**: 2026-09-16
- **Evidence**: `core/src/{permission,plan}.rs`, `core/src/runtime/{planner,trust,lint,pack}.rs`,
  `cli/src/{trust,lifecycle_plan}.rs`

## Context

ADR 0041 requires target-local capability declarations because exact App, Shell, and Sys targets
must not inherit unrelated permissions. Arbitrary scripts cannot always be described completely,
however, and forcing personal Preset authors to maintain a speculative exhaustive list encourages
misleading empty declarations. Field-level wildcards would also have incompatible meanings: a
command wildcard is not an environment-input allowlist, and a filesystem wildcard cannot grant
administrator execution safely.

The ergonomic choice must remain separate from trust and approval. A Preset declaration is still
an author claim, external code trust is durable local review state, and a security Plan approval is
one-shot mutation authority.

## Decision

Permission declaration schema v2 adds `opaque_code = "unrestricted"`. It normalizes to one
`opaque-code/unrestricted` review identity instead of expanding into every known permission kind.
That identity covers otherwise undeclared command, filesystem, network, and system effects of an
opaque-code surface. It never covers environment input identity or administrator authorization;
executor allowlists, secret sensitivity, typed elevation, ownership, and snapshot-bound approval
remain explicit.

The common declaration works for App category code, Shell commands, and Sys script/profile code.
App lifecycle planning adds it only when a generator, hook, artifact, or teardown is triggered.
Declarative App files, managed Sys resources, and typed Sys package bootstrap do not inherit an
untriggered opaque-code marker. Shell installation presents the marker as the risk of installing
the selected command.

Shell and Sys accept a category-level `[permission_defaults]` declaration. Parsing resolves the
default independently into entries that omit their own permission table; an entry-level table
replaces the default. Runtime Plans and trust grants remain target-local after resolution.

External Shell commands using unrestricted opaque code join the trust target grammar as
`shell/<category>/<command>`. Existing enumerated external Shell commands retain their established
behavior, including live development semantics. The special `preset` trust target batches every
current App, unrestricted Shell, and Sys requirement for review, enrollment, or revocation, but
stores separate exact grants. It does not trust future targets or changed code.

`preset new <kind> --unrestricted` scaffolds the new declaration. Lint emits the stable advisory
`unrestricted_opaque_code`. Preset bundle schema v2 records `unrestricted_opaque_code` so consumers
can identify the broader risk without interpreting a wildcard.

## Consequences

- Authors can describe genuinely arbitrary personal code honestly without enumerating speculative
  capabilities.
- Unrestricted code remains visibly broader than an explicit empty or enumerated declaration.
- Ambient environment and elevation cannot be acquired through the unrestricted marker.
- Category defaults reduce repetition without changing targeted Plan or grant scope.
- Trusting a current Preset collection remains a batch UX over exact grants, not mutable-directory
  trust.
- Bundle schema v1 consumers must add support for the v2 risk field before consuming new bundles.
