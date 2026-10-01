# 0094 — Security Plan filesystem review summaries

- **Status**: Accepted
- **Date**: 2026-09-29
- **Evidence**: `core/src/{plan.rs,runtime/planner.rs}`, `cli/src/lifecycle_plan.rs`
- **Extends**: ADR 0078, ADR 0086, ADR 0091

## Decision

Core records filesystem presentation provenance at the typed operation derivation site: user
resource, installed output, maintenance, or recovery material associated with a logical target.
The optional `PlanV1.filesystem_review` groups contain exact permission identities and participate
in the existing Plan fingerprint. Empty groups are omitted, and old Plans default to no groups.
This additive review metadata retains schema v2, as with permission scopes; it changes no receipt,
journal, trust schema, permission resolution, or approval authority.

All CLI Security Plan entry points default to concise permissions and accept `--verbose` for
complete paths and identities. The command future scopes the display preference with a Tokio
task-local value; it is not persisted configuration or a process-global switch. Existing upgrade
and bootstrap options retain their behavior and use the same permission summaries. Bootstrap
continues to preserve target-local versus shared scope attribution.

Only uniquely attributed, non-executable filesystem effects in ready, non-recovery Plans are
summarized. User resources, missing/conflicting attribution and groups containing unplanned
permissions remain explicit. Recovery and blocked Plans show every permission path. CLI rendering
never infers ownership from `shine:` or a filename suffix. Steps with exceptional diagnostics, author statements,
code boundaries, administrator access and non-filesystem capabilities remain visible.

Within each displayed scope, the CLI merges Shell command installation effects using typed code
boundaries, shows profile installation as Shell integration, and combines internal maintenance and
recovery material. Counts deduplicate paths and command targets. Recovery associations with explicit
user destinations stay visible; verbose output retains every permission. This grouping does not
merge bootstrap scopes or change Core classification or authorization.

## Consequences

Full required permissions still bind approval and are checked after fresh planning. Changing
presentation provenance invalidates an earlier approval. An owned state directory is never a
blanket grant; preset code remains unisolated. Tests cover real Shell transactions, conservative
fallback, full rendering, command flag coverage and fingerprint binding.

## No-op review refinement

Core derives App destination permissions after file assessment; manual generators skipped during
upgrade contribute no destination effect. Managed Sys requirements are accumulated per item and
omitted only after an unchanged result. Observations and author statements remain captured, and
shared transaction permissions are not removed with a no-op item. This is a planning correction,
not display filtering or implicit authorization. Lifecycle summaries omit snapshot identities and
a closed list of routine transaction codes; unknown, preserve and blocked diagnostics stay visible.
Verbose review retains the full Plan.
