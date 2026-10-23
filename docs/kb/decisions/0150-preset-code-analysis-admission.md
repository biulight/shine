# 0150 — Snapshot-bound App code analysis admission

- **Status**: Accepted; implemented and validated
- **Date**: 2026-10-22
- **Partially supersedes**: ADR 0091 (optional author statements)
- **Extends**: ADRs 0040, 0080, 0092
- **Requirements**: [Code analysis PRD](../../preset-code-analysis-prd.md)

## Contract

Supported App Bun code receives automatic static analysis. Admission is binary: any definite
declaration difference or analysis failure blocks; coverage gaps and uncorroborated declarations
do not. All diagnostics coexist. Neither a declaration nor an analysis result grants trust,
permission, elevation or approval. No diagnostic means silence in ordinary output, not a safety
claim. Unsupported native code retains a visible coverage gap and existing trust requirements.

Core owns entry selection, using the same typed generator, hook, artifact and teardown conditions
as execution. Missing `when_env` keeps explicit evaluation's static fallback; implicit upgrade
skips `auto = false`. Refresh retains its missing-input error. Trust failure does not exclude an
entry. Receipt-only cleanup does not require absent source. Replanning must discover newly enabled
entries. Lint deliberately selects all supported entries across supported platforms; author Plan
and fixtures select only entries under their captured hypothetical inputs.

## Evidence preparation and use

Keep analysis within Core, separate from the planner's resource observation. A command-owned
session holds immutable source facts and cumulative budgets. Source cache keys include exact
logical source/dependency identities, parser mode and rule/configuration versions. Entry
comparison additionally binds the typed entry, normalized author declarations and effective
environment contract. Shared source facts never imply shared admission results.

Planning discovers typed entries through its existing trigger decisions. Orchestration prepares
their evidence outside lifecycle locks; final planning consumes evidence and captures fresh state.
Missing evidence is a blocker, never an empty report. Execution-time replanning only verifies
prepared evidence. An expanded entry set requires preparation and renewed review outside the lock.
Do not parse source inside each `plan_*` call or infer entries from diagnostic/resource strings.

Explicit generator evaluation prepares and checks evidence after trigger selection but before
materializing an invocation or starting a process. It retains persistent trust and creates no
lifecycle approval. The common App script launcher checks the typed entry's evidence again before
its first write. Evidence remains private, non-deserializable Core data; frontend reports are
projections and cannot be submitted as execution evidence.

Source facts record environment read names only. Core supplies source-to-target mappings,
source sensitivity and the exact fixed-variable set without values. Fixed variables override
injection aliases as in the executor; no prefix exemption exists. Unknown source identity remains
a gap even if a same-name declaration exists. Assignment/delete targets are not reads; compound
assignment and updates are reads as well as writes. Mapping changes invalidate comparison and
approval, without requiring source reparsing.

## Parser decision

Use pinned `tree-sitter = 0.25.10` and `tree-sitter-typescript = 0.23.2`, with a JavaScript
grammar for JavaScript entrypoints. The parser and grammar are MIT licensed. Tree-sitter declares
Rust 1.76; the TypeScript grammar does not declare an MSRV, so an actual Rust 1.90 build is required
before completing the core stage. The maintainer authorized raising the workspace MSRV to
Rust 1.90 to use the latest `tree-sitter-language` 0.1.9 (MIT, Rust 1.90). Grammars compile locally with the existing C build toolchain;
analysis itself never launches a compiler, external analyzer or dependency installer.

Tree-sitter exposes byte ranges, row/byte-column locations, error/missing nodes and
`Parser::parse_with_options` with a progress callback. Reject recovered syntax errors instead of
treating a partial tree as complete. Convert locations to a documented one-based UTF-8 byte column
without platform-dependent path conversion. Walk trees iteratively with a node budget. Retain no
parser error text or source excerpts in public reports.

Oxc 0.75.1 was also inspected: MIT, Rust 1.86, typed JS/TS AST and separate semantic analysis.
Its inspected parser API does not offer the progress callback used by this design. Tree-sitter's
interruptible parsing is preferable for hostile input and command-level cancellation; the cost is
maintaining a deliberately limited binding resolver rather than using a complete semantic engine.
This choice must be validated with alias/shadowing and malformed-input tests, not just parse demos.

Evidence: published manifests and Rust sources for
[Tree-sitter 0.25.10](https://docs.rs/crate/tree-sitter/0.25.10),
[TypeScript grammar 0.23.2](https://docs.rs/crate/tree-sitter-typescript/0.23.2), and
[Oxc 0.75.1](https://docs.rs/crate/oxc_parser/0.75.1).

## Rule boundaries

Recognize calls/member expressions, never words in comments, strings or variable names. Resolve
builtin imports (including explicit aliases) and unshadowed globals. Unsupported aliasing,
reassignment or ambiguous binding produces a coverage gap rather than a definite capability.
Literal arguments may be compared; dynamic targets remain gaps. Do not evaluate expressions or
read ambient environment values. Each implemented rule must list supported syntax and tests.

- `process.env` and `Bun.env`: static property/string keys are reads under the entry contract;
  dynamic keys and unsupported destructuring/aliasing are gaps.
- Node filesystem and Bun file APIs: distinguish read/write/remove. Compare only identities
  expressible by existing path-base semantics. No new globs or inferred directory inheritance.
- `fetch`: static HTTP(S) hosts compare using existing network declarations; dynamic URLs are
  gaps. Never render URL credentials, paths, queries or fragments.
- Node/Bun subprocess APIs: literal executable identity can compare with `commands`; every call
  also reports unanalysed subprocess effects. Shell execution and dynamic commands remain gaps.
- Imports: recurse only into unambiguous category-local relative modules. Missing modules,
  escaping paths, unsupported extensions, packages and remote modules get stable diagnostics.
  Cycles terminate through visited identities. Do not use host Node resolution or package install.
- `eval`, dynamic loading and unsupported runtime calls retain explicit gaps; absence of a
  recognized rule is never proof of absent effects.

## Failure scopes and budgets

Use three internal failure scopes, not three admission states. Declaration differences belong to
one entry/contract. Parse errors and local size/depth failures belong to the source/dependency
graph and affect all dependants. Cancellation, cumulative budget exhaustion and unrecoverable
analyzer faults terminate the command and forbid further process starts. Do not catch a terminal
failure as an ordinary file error or a successful negative fixture assertion.

Initial limits to validate against real presets and stress fixtures: 1 MiB per source, 256 modules
and depth 32 per dependency graph; 16 MiB, 2,048 unique source files, 50,000 findings and 2,000,000
visited syntax nodes per command. Parser progress also has a cumulative work budget. Source cache
hits do not repay or reset work; producing per-entry comparison diagnostics consumes its own
bounded capacity. Final thresholds and observed sample sizes belong in implementation evidence.

Normal cancellation uses existing process supervision/cleanup. Forced cancellation may leave an
invocation as permitted by ADR 0092; later work must never reuse it. No new cleanup framework or
cross-file rollback is introduced. Completed per-file results may remain visible; unfinished work
cannot be reported as matched or successful.

## Compatibility and delivery gates

Use an internal analysis schema/rules version 1. Introduce explicit analysis identity/diagnostic
projection in the next Plan/approval/review contract version (3), author Plan version 3, lint
report version 2 and fixture report version 2. Keep old fixture input version 1 readable; report
version and fixture input version must become separate constants. Existing journal, receipt,
trust and permission-declaration schemas do not change solely for analysis. Old approvals cannot
authorize the changed contract. Public reports contain safe locations, codes, scope and opaque
identities, not executable evidence or raw literal values.

The latest stable baseline at design time is `v2.6.0`. Mandatory declaration admission is an
incompatible behavior change under the repository's versioning rules and therefore requires a
major release unless the maintainer explicitly grants a separate exception. Prior minor-release
exceptions do not cover this feature. Implementation does not authorize tagging or publication.

The maintainer subsequently chose 2.7.0; the
[2026-10-24 compatibility exception](../conventions.md#versioning) covers metadata, report,
Rust library source and admission changes. Consumers constructing public structs must adapt
to added analysis fields as well as the serialized schema versions.

Stages B–D must establish parser/alias/dependency/privacy tests, same-scope author/runtime parity,
all execution entry gates, stale evidence/approval rejection, cache reuse, accumulated budgets,
partial evaluation and cancellation. Retain existing ownership/lock/recovery regressions.
Before delivery, update both manual locales, author skill, architecture KB and built-in declarations.
Stage E is a measured scope decision, not a requirement to add languages or split a package.
