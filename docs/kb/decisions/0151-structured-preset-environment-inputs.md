# 0151 — Shell and App inputs declare mapping and sensitivity together

- **Status**: Accepted
- **Date**: 2026-10-22
- **Evidence**: `core/src/env.rs`, `core/src/runtime/{app_metadata,shell/metadata,preset_migration}.rs`

## Decision

Shell Bun entries and App generators, hooks and artifacts accept only structured `env` entries:
`source`, optional `target` (defaults to source), and required `plain`/`secret` `sensitivity`.
Core resolves these into the existing ordered source/target mappings and effective environment
permissions. Duplicate targets and conflicting source classifications fail. Same-source aliases
with consistent sensitivity are allowed; category permissions never widen an entry's input list.
Additional explicit environment statements remain valid and must agree with structured inputs.
Shell entry permissions still replace category defaults as a whole before input permissions are
derived; inputs never inherit capabilities from another command.

Normal parsing rejects legacy string arrays and provides migration guidance. Only `preset migrate`
joins old mappings with explicit effective permission sensitivity. It preserves aliases, conditions,
unrelated statements and metadata formatting, removes converted redundant entry/category statements,
and retains shared Shell defaults and App statements when a generator can be filtered by platform
or destination. Conversion visits only executable entry owners; ignored root/file fields cannot
consume category permissions. Missing classifications, invalid mappings or mixed arrays need manual
correction. The existing candidate validation, backup-first review, source rechecks and
managed-overlay restrictions apply. No global startup scan, in-memory legacy fallback, automatic
source rewrite, secret decryption or trust enrollment is introduced.

## Consequences

Old-format knowledge is confined to metadata migration, while normal parsers use one input model.
App code analysis and Plan generation consume the same derived category contracts. Plain hashes,
opaque secret identities, approval freshness, trust and entry-local execution keep their existing
boundaries. Shell still resolves current configured values per invocation; App execution does not
gain Shell's encrypted-first lookup. Installed launchers, `env run --with`, manifests, Action IR,
journals and recovery retain their string mapping protocols and do not require migration.
Metadata byte changes invalidate external snapshot trust normally; migrate does not renew it.

This deliberately trades a temporary source-conversion path for eliminating permanent dual-format
parsing and precedence rules. Migration code and its regression fixtures count toward maintenance
cost; reduced user metadata is not evidence of lower total line count.
