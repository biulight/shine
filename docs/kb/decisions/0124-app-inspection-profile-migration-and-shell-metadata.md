# ADR 0124: Separate App inspection, profile reconciliation and Preset metadata phases

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

App inspection combined generator policy, host observations and status projection. Sys profile
reconciliation combined pure merging with process and file effects. Preset migration mixed input
discovery, candidate validation, display and mutation. Shell category parsing mixed explicit
metadata and compatible native discovery, obscuring admission order.

## Decision

- Use named App assessment policies for install preview, install, upgrade and explicitly evaluated
  inspection. Keep snapshot-only assessment separate from generator execution. Ordinary inspection
  requires only filesystem observation; retain explicit evaluation trust checks, assessment before
  installed-content reads, status precedence, diagnostics and lifecycle failure semantics.
- Separate Sys phase input reads/EOL normalization, ordered reconciliation writes, bounded Git
  adaptation and the pure fallback merge. Keep phase order, force backups, conflict artifacts,
  missing-base handling, LF comparison/unchanged-byte preservation and Git fallback rules. Do not
  introduce a transaction or combine Shell and Sys sentinel handling.
- Keep Preset migration approval and sequencing in the CLI facade. Return named discovery inputs;
  separate same-scope candidate assessment, report/diff rendering and backup/apply effects. Preserve
  default-No approval, managed-overlay diagnosis-only policy, all captured source observations
  (including shadowed base files), complete private backup before apply and immediate per-file
  checks. Partial independent edits and retained backups keep their existing failure behavior.
- Extract snapshot-only Shell metadata parsing with separate explicit and discovered file paths.
  Preserve defaults validation, platform filtering before runtime validation, native shell filtering
  before remaining entry validation, command sorting/duplicates and typed diagnostic origins.
  An explicit empty `files` list continues to suppress discovery. Entry permissions replace defaults
  as a whole; they never inherit omitted opaque grants. No Host is needed for parsing.

## Consequences

Public commands, DTOs, Plan fingerprints, receipts, journals and safety gates retain their contracts.
The ordinary App inspection Host bound is narrower; it grants no new execution or mutation authority.

Regressions cover observation-only App inspection and generator policy differences, profile conflict
preservation/Git fallback/CRLF merge/write-failure ordering, migration source changes after backup
and final per-file checks, plus Shell filtering order, explicit-empty discovery and diagnostic kinds.
Native Windows execution remains covered only by platform-specific tests, not virtual Host evidence.

Review regressions also bind selected multi-file evaluation before installed-content reads, continuing
after one generator fails without evaluating excluded categories. They exercise Git clean/conflict
outputs and non-NotFound spawn errors through phase reconciliation, real CLI backup failure before
any metadata edit or runtime initialization, and permission overrides through production Shell parsing.
