# Preset code analysis implementation

Implemented for App Bun JS/TS and local dependencies. Expansion to other languages or a separate
package is deliberately deferred; see the scope decision below.
Contract: [ADR 0150](../decisions/0150-preset-code-analysis-admission.md).

## Source rules, version 7

`core/src/analysis/` accepts captured category-relative source bytes, parsed declarations and an
environment identity contract. It has no filesystem/process Host, ambient environment lookup,
network client or write API. URL parsing is local. `Session` caches source facts while comparing
each entry's declarations and environment independently. Cache hits preserve cumulative budgets.

The parser uses pinned Tree-sitter 0.25.10, JavaScript grammar 0.23.1, TypeScript grammar 0.23.2,
and language bindings 0.1.9. The maintainer authorized Rust 1.90 for the latest language binding.
Recovered/missing syntax nodes block with safe positions; raw parser text is never exported.
Locations are one-based lines and UTF-8 byte columns. Public diagnostics contain category-relative
paths, behavior classes, codes and optional opaque target hashes; no source excerpt or raw target.

| Rule | Supported syntax and comparison | Explicit limitation |
| --- | --- | --- |
| Environment | `process.env.NAME`, `Bun.env.NAME`, literal string subscripts; entry source/target/sensitivity mapping; exact fixed-variable names | Dynamic keys, unknown source identity and whole-env-object access are gaps; plain assignment/delete is not a read, compound assignment/update is |
| Network | Unshadowed `fetch`, `globalThis.fetch` (including literal subscript), and supported immutable aliases, literal HTTP(S) URL; URL-normalized host compared with exact host or `any`. Node `https.get` uses the same comparison for a literal HTTPS URL alone or with an inline function callback | Dynamic URLs, unsupported string escapes and ambiguous callbacks/options for `https.get` are gaps; credentials, URL path/query/fragment never appear in reports |
| Node files | `readFile[Sync]`, `createReadStream`, `writeFile[Sync]`, `appendFile[Sync]`, `createWriteStream`, `unlink[Sync]`, `rm[Sync]`, `rmdir[Sync]`, `copyFile[Sync]`, `rename[Sync]`; supported `fs.promises` forms | Literal paths only; exact existing logical permission identities, no invented globs/inheritance; ambiguous native path spellings are gaps |
| Bun files | `Bun.write`; `text/json/arrayBuffer/bytes/stream/exists/delete/writer` on a direct `Bun.file` result or a tracked `const` handle | Constructor sites distinguish handles even for the same path. Saving a handle retains the object coverage gap. Only recognized method calls produce access facts, and dynamic paths remain gaps |
| Commands | `Bun.spawn[Sync]`, Node `spawn[Sync]`, `execFile[Sync]`, `fork`; literal executable or first literal array element | `fork` module paths and Node options are not treated as definite executable identities; every subprocess retains an independent effects gap; dynamic executable adds an identity gap; shell `exec[Sync]` is dynamic code |
| Dynamic code | Unshadowed `eval`, dynamic `require`, `import`, recognized Bun shell invocation | No execution, remote module download or recursive subprocess analysis |
| Bindings | Builtin namespace/default/named imports and named import aliases; immutable `const` aliases including forward references in closures; parameter/block/function/generator/catch/loop shadowing | Ambiguous/reassigned bindings and unsupported calls are gaps; member mutations propagate across supported object aliases without invalidating unrelated members or previously captured function values; no general control-flow analysis or interpretation of arbitrary values |
| Dependencies | Static imports/re-exports and literal `require`; category-relative paths, cycles, exact filenames or unique extension/index candidate | Escapes, missing/ambiguous dependencies, unsupported builtin APIs/languages and packages have stable gap codes; no host module resolution |

Rule version 3 includes generator declarations as lexical bindings and function scopes. Alias
resolution uses at most 64 fixed-point passes, charged to the command node budget; cycles and
unresolved chains remain gaps. Member mutation identities are collected before invalidating names.
An initializer preceding a mutation in the same function may retain the original member value;
cross-function mutation order remains unknown. Dynamic property writes invalidate members of that
object conservatively. This keeps replaced methods unknown without hiding unrelated members or
functions already captured before replacement.

Rule version 4 extracts binding names only from binding positions, excluding default expressions,
computed keys and complete TypeScript type annotations. Destructuring assignments use the same
pattern traversal to invalidate their actual identifier/member targets. Default expressions still
participate in runtime behavior analysis; nested/rest patterns preserve real local shadowing.

Rule version 5 repairs four issues found by the v4 sample assessment. Argument and executable-array
selection skips comments while retaining holes/spreads as unknown. Reads and assignments resolve
the same bounded lexical binding identity; unrelated local names do not invalidate outer aliases,
and shadowed `globalThis`/`require` do not change global classification. Function-body declarations
do not shadow the outer environment used by default parameter initializers. CommonJS return-value
recognition checks the loader's binding and mutations, preserving values captured before replacement
but retaining a gap for replaced loaders. The rule bump invalidates old analysis identities; report,
trust and approval schemas and the non-blocking coverage-gap policy are unchanged.

Rule version 6 closes the follow-up syntax boundaries: namespace-import and assignment-target
comments do not hide identities; TypeScript write wrappers inspect only the operand, not type
names. Non-declaring `for in/of/await` targets are writes to existing bindings/members, while loop
declarations retain their scopes. Method parameters exclude computed keys, and class static-block
vars stay inside that block. Loader calls captured before replacement retain dependency analysis;
immutable loader aliases retain builtin return identity. Replaced/shadowed loaders and uncertain
cross-function mutation order still produce gaps rather than definite builtin effects.

Rule version 7 tracks `Object.assign` writes to supported builtin objects and their immutable
aliases. Literal source keys invalidate individual members. Dynamic keys, spreads and opaque
sources can replace any member of the target. Previously captured function values retain the
existing same-scope ordering rule. The assign call itself retains its coverage gap.

Version 7 also compares both paths in Node copies and renames. A copy reads its source, a rename
removes the source name, and both write the destination. Each literal is compared independently.
A spread can make the following argument positions unknown. The promise module shares member
identity with `fs.promises`, including replacements, and only exposes promise forms of these APIs.

Identifiers use decoded Unicode spellings for declarations, references, member keys and writes.
URL literals and string member keys decode standard JavaScript escapes, including UTF-16 pairs
and line continuations. Legacy numeric escapes, unpaired surrogates and decoded controls remain
unknown string targets. Other path, command and module literals retain their existing escape
limitations. Decoding leaves source bytes and diagnostic locations intact.

### Shared semantic boundaries

The v7 review groups corrections by semantic role rather than adding a rule for each source
spelling. Keep the pinned parser and existing API whitelist. No new production module, semantic
IR or external compiler is needed for these local decisions.

| Boundary | Owner | Required behavior |
| --- | --- | --- |
| Type syntax versus runtime | Syntax traversal and `is_type_syntax` | Validate and budget every node. Pure types create neither value bindings nor effects. Ambient declarations retain unknown value bindings but have no runtime effects. Initializers, enum/namespace bodies and runtime computed keys remain analyzed. |
| Equivalent operands | `wrapped_operand` | Parentheses, `as`, `satisfies`, non-null/angle assertions and generic instantiations select the same runtime operand for calls, aliases, literals, receivers and write targets. Exclude their type operands from all runtime passes. |
| Names and lexical identity | Binding collection and `Bindings::visible/member` | Decode identifier and quoted import names; normalize named default imports without treating literal dotted names as paths. Unknown imports, import aliases, enums, namespaces and ambient value declarations still shadow globals in their own scope. Namespace bodies contain their own `var` bindings. Type-only names do not. |
| Read/write roles and mutation | `pattern_targets` and the mutation pass | Collect write-only targets and mutations from the same pattern traversal. Plain/destructuring assignment, loop targets and deletion are not environment reads. Computed keys, default RHS values, compound assignment and update still read their operands. |

The additional state separates binding collection from runtime passes; it does not add another
analysis pipeline. Keeping the former per-call checks would duplicate type erasure across effect
handlers and write-role logic across mutation and environment analysis. The shared owners remove
that drift. Existing source-location, privacy, cache, budget and lifecycle tests remain independent.

The maintained [semantic matrix](../../../core/src/analysis/tests/semantic_matrix.rs) compares
admission and diagnostic identities across spelling/wrapper/alias variants, known and dynamic
member writes, capture timing, read/write patterns, type erasure and lexical shadowing. Repeated
advisories and source positions can differ between equivalent layouts; separate tests retain
count/location assertions. Core lifecycle cases verify both approval acceptance and rejection.

Keep these limits explicit:

- Unknown module spellings and `import = require(...)` retain dependency gaps; registering their
  local names does not resolve their values or expand the dependency graph. Namespace export
  values and internal import aliases are not interpreted.
- Arbitrary expression evaluation, destructured value propagation, function returns and general
  control-flow analysis remain outside the resolver. Same-function capture ordering retains the
  existing syntactic rule, not a proof about loops, reachability or all executions.
- Mutation coverage remains direct/pattern writes and supported `Object.assign` calls. Reflective
  writes and arbitrary call side effects are not made complete by this audit.
- Dynamic URLs/options, unsupported escapes and paths, detached BunFile methods, unknown packages
  and other existing gaps remain advisory. Bounds and parse failures still block; this is not a
  TypeScript type checker or a proof that arbitrary JS/TS code is safe.

For future syntax changes, first add an equivalent transformation and a shadowed/rewritten or
unknown-value control to the relevant matrix axis. Fix a shared owner when an invariant fails;
record an explicit gap when the value cannot be established. Expand the API or module-resolution
scope only as a separate decision. These unreleased corrections keep rule version 7.

Regression coverage includes literal/comment and dynamic/spread pairs, local versus outer mutation,
JS/TS default parameter scopes, replaced versus captured loaders, and author fixtures for all three
simulated platforms with and without code trust. Blocking cases also exercise Frontend explicit
evaluation, rejecting before invocation writes or process starts without a lifecycle approval.

Uncorroborated declarations do not block. They also do not remove gaps. Known command identity
plus a subprocess gap remains incomplete even if file/network declarations are supplied.
Only difference and failure diagnostics block, and all diagnostic classes can coexist.

### Diagnostic presentation

P2 aggregation is confined to CLI text rendering. Ordinary reviews group repeated coverage gaps
by code and behavior within one entry, showing the original diagnostic count and first location.
For a failed generator in compact global `update`, identical nonblocking reports for the same
category and script can share a summary listing every affected source. Known coverage gaps use
plain-language counts with full locations in `--verbose`. Blocking findings stay per entry.
A generator trust denial names the non-embedded layers contributing effective files anywhere
in that App category, even when the generator script itself comes from the embedded base.
Compact lifecycle Plan review lists all entries sharing the same script, target and identical
nonblocking report together within each Plan. Coverage notes describe analysis limits; the Plan
prompt describes one-operation consent. Detailed review and author Plan rendering retain individual
diagnostics and opaque identities.
Explicit inspection uses the existing display modes: `info --verbose/--diff`, detailed `update`
views and `app info` retain every evaluated location. This changes only rendering of captured
reports; no display flag enables generator evaluation or changes its trust requirements.
Lint text groups only `code_analysis_*` warnings with the same target, code and message, retaining
an example resource including its entry label. Non-analysis warnings and all errors stay individual.
The text points to lint JSON for all checked source entries; that scope can differ from a runtime
review's triggered entries and environment contract.

Core reports, JSON, diagnostic ordering/counts, schema/rule versions, admission, evidence binding,
trust and approval are unchanged. Lint `valid`, `clean`, `complete`, summary and `--deny-warnings`
still consume the full Core report. Grouping never changes partial/terminated analysis into a
complete result, and renderers encode diagnostic operands after matching their original identities.

Compact-rendering checks retain byte-identical JSON while reducing repeated text. Renderer and
real CLI regressions cover distinct behavior groups, error retention, warning exits, encoded
locations and pre-execution rejection.

Follow-up review found that explicit inspection had hard-coded compact analysis rendering even
in detailed views. Real CLI regressions reproduced the missing expansion before repair and now
check compact/detailed output, error retention and pre-execution rejection at the public entry
points. Advisory-only untrusted code also remains blocked in both modes; lint regressions retain
raw JSON targets while encoding control-bearing category names in grouped text.

## Resource limits

Defaults are 1 MiB per source; 256 files and depth 32 per dependency graph; 16 MiB and 2,048 unique
source files per command; 50,000 accumulated facts/comparison diagnostics, 2,000,000 syntax/binding work units
and 100,000 parser progress callbacks. Byte, file and parser work are charged on cache misses;
per-entry diagnostic work is still charged on cache hits. These are conservative work limits,
not a time/memory guarantee or a claim of exhaustive analysis. Binding lookup is indexed by name;
more than 128 candidates for a name is unresolved rather than allowing unbounded scope matching.

Function/namespace binding-scope walks charge the same node-work budget and check cancellation.
Ancestor results are cached only within one parsed tree, so sibling declarations do not repeat a
deep parent walk. Function capture boundaries remain separate from namespace binding boundaries.
The `binding_scope_*` regressions cover budget exhaustion/terminal reuse and bounded sibling
lookup without leaking namespace variables. Counting only the initial tree walk missed this work;
the extra namespace pass in the initial v7 implementation made the existing deep-block cost worse.

Local file/graph limits and syntax errors produce blocking reports that other independent entries
can continue past. Cancellation and cumulative limits latch a terminal failure on the session,
including for later cache hits. Parser interruption and iterative node traversal check that state.
Core orchestration must propagate it before scheduling more source work or script processes.

## Rule coverage and limits

Manually labelled rule fixtures cover literal host/path/command differences, import aliases,
shadowing, source-to-target mappings, env writes versus reads, unknown subprocess effects,
dependency cycles/missing/escape/ambiguity, parse failures, privacy and cumulative limits.
Built-in Surge and Clash Verge samples exercise production source parsing and deterministic
source-cache reuse. Their dynamic targets and user-defined calls retain explicit gaps.
Comparisons require each entry's actual environment contract; sample diagnostic counts are not
a security/coverage percentage or proof that declarations match.

The v6-to-v7 acceptance comparison used identical built-in source bytes and Core-derived contracts
for all seven Bun entries. Admission was unchanged. The only diagnostic change was Surge's dynamic
`rename` call, replacing one unknown-call gap with separate source/destination path gaps in both
artifact entries. Literal-path controls verify that definite source/destination differences still
block. This supports retaining the current API/parser scope; the small corpus does not establish
coverage of arbitrary presets. Further changes need a reproducible supported-scope regression or
a separately justified scope expansion.

Do not describe the bounded analysis as exhaustive enforcement of script behavior. Pure analyzer
tests do not replace consumer-boundary tests or native-platform checks.

## Core preparation and author integration

`runtime/code_analysis.rs` owns a private command session, prepared entry reports and missing-work
registration. The analyzer accepts borrowed source bytes; Core does not copy an entire category's
payload bytes per entry. The immutable snapshot digest is computed once per runtime. Frontend human
review and its consumed approval carry the same private session across fresh runtime captures;
changed source/metadata/contracts still require a different evidence key.

Public App planning first observes typed triggers. If evidence is missing, its orchestration prepares
it outside the lifecycle lock and re-observes a fresh Plan. Locked approved methods call only the
`*_prepared` planners. They do not run an extra pre-lock state observation, nor analyze code while
waiting for or holding the lock. Missing/newly triggered evidence keeps the locked Plan non-ready;
the caller must prepare and review again outside the lock. Static operations retain their original
lock timing. The existing lock-wait regression caught and prevented an extra pre-lock observation.

Plan/approval/review schema 4 binds a compact analysis identity plus safe scope reports. Author Plan
schema 3 omits that binding identity, lint report schema 2 reports analyzed entry labels, fixture
report schema 2 adds analysis diagnostic assertions and explicit unfinished cases. Fixture input
schema 1 remains readable. Generated schema reference version 5 names the new report schemas.
Author Plan exit success still describes valid hypothetical planning; `ready` expresses admission.
Lint differences/failures are errors, gaps are warnings, and fixture negative assertions may pass
only for completed cases. A terminal analysis failure cannot be absorbed as an expected blocker.

## Execution admission and cancellation

Explicit generator evaluation prepares evidence before trust checks and snapshot materialization.
A file-local difference or parse failure becomes an evaluation failure while independent selected
files continue. Ordinary inspection still defers generators, and an absent `when_env` retains static
fallback. The generator, artifact and hook launch paths require the exact Core-owned evidence;
`run_app_script` checks again before its first invocation-directory write. Native hooks retain a
language coverage gap and use the same command cancellation check.

Preparation runs on a blocking worker over the shared immutable snapshot. Dropping its awaiting
future sets the analyzer's cancellation flag; parser callbacks and traversal stop subsequent work.
An interrupted child or command cancellation latches the same session state. App process waiting
uses the existing Host process-future drop/ProcessScope cleanup; the script launcher then attempts
its existing invocation cleanup. Approved App methods propagate terminal state even when an
ordinary hook error would be non-fatal. `ProcessOutput.interrupted` distinguishes an actual child
interrupt/termination from an ordinary failure exit; custom Hosts should set it when applicable.

Cancellation regressions cover dropped preparation, child interruption, graceful real-process
cancellation and forced task abort. Forced abort may retain the old invocation, while the next
operation creates a different invocation and never executes from the retained directory. The
inherited-terminal regression also checks signal classification and foreground restoration.

## Acceptance evidence index

The PRD scenarios below are separate assertions, not a claim that static analysis proves safety.
Pure rule tests live in `core/src/analysis/tests.rs`; admission tests live in
`core/src/runtime/code_analysis/tests.rs`. Existing lifecycle/lock/recovery tests remain intact.

| PRD | Evidence |
| --- | --- |
| 1 | `literal_network_matching_is_silent_and_difference_is_located`; shared CLI renderer omits empty reports |
| 2 | `launcher_rejects_missing_evidence_before_snapshot_write`; CLI installation/evaluation rejection test |
| 3 | Literal network/filesystem rules assert missing scope and one-based source locations |
| 4 | `declarations_and_new_source_invalidate_approved_evidence`; changed mapping/sensitivity test |
| 5 | `code_analysis_blocks_supported_scripts_before_execution_in_every_cli_evaluation` grants trust and attempts `--yes` |
| 6 | Relative dependency, dynamic call and subprocess tests retain coverage-gap diagnostics |
| 7 | Parse failure, local-limit, terminal-budget and native-language reports have distinct codes |
| 8 | Pure analyzer owns no Host/environment/process/network APIs; author/runtime tests assert observation-only effects |
| 9 | `plan_prepares_once_and_locked_planning_only_consumes` checks parser counts across replanning |
| 10 | Existing full-suite approval consumption, lock-wait, recovery and observation-only regressions |
| 11 | Untriggered generator and existing source-missing receipt/uninstall regressions |
| 12 | Same pure location/path assertions run in native macOS, Windows and Linux builds |
| 13 | Pure privacy test, safe Frontend serialization and CLI private-literal rejection assertions |
| 14 | Existing shared-resource/selector planning tests plus typed entry registration in those planners |
| 15 | `definite_difference_blocks_approval_with_gaps_retained` |
| 16 | Core launcher, direct Frontend and all CLI evaluation forms reject before invocation writes |
| 17 | Independent-file evaluation test, existing receipt/destination/trust regressions and CLI evaluation test |
| 18 | `environment_mapping_is_per_entry_and_source_cache_is_shared`; missing-source-sensitivity rule test |
| 19 | Exact fixed-variable exemption, unknown/dynamic env identities and env write/delete tests |
| 20 | Shared-source independent-entry test, failed-source cache test and changed mapping/sensitivity evidence test |
| 21 | `subprocess_effects_remain_unknown_when_command_matches`; fork/option identity regression |
| 22 | `untriggered_generators_do_not_parse_invalid_source`; existing refresh input/assessment policy regressions |
| 23 | Changed mapping/sensitivity/newly-triggered test; untrusted entries are prepared before trust refusal |
| 24 | `author_tools_share_diagnostics_and_negative_fixtures_pass_without_execution`; terminal fixture test |
| 25 | `lint_checks_entries_skipped_by_runtime_trigger_conditions`; author/fixture scope and no-effect assertions |
| 26 | Cumulative budget, interrupted generator, dropped preparation, real process cancel/abort and unfinished fixture tests |
| 27 | `advisory_diagnostics_coexist_without_granting_or_blocking_admission`; all four diagnostic kinds coexist |

The full-suite check exposed a pre-existing planner/executor mismatch for replacement installation:
`--replace-managed` rewrites unchanged App files and triggers their hooks. Planning now records the
replacement and prepares the hook evidence instead of incorrectly classifying the file as unchanged.
The existing CLI hook regression covers this behavior; execution was not weakened to accept missing
analysis. Existing CLI generator fixtures that deliberately write a marker now declare that write.

## Scope decision (stage E)

The initial built-in assessment found unknown calls to be the dominant gaps in Surge and Clash
Verge, followed by dynamic targets and unknown environment identities. No definite declaration
mismatch was observed with actual entry environment contracts. Repeated diagnostic contexts do
not measure a coverage percentage.

Keep the first release limited to App Bun JS/TS and local dependencies. Additional languages would
not resolve the dominant dynamic-target/call gaps and would add parser, binding and platform costs
without corresponding evidence of value. Do not split `shine-analysis` yet: the current consumers
share Core's snapshot, declaration and input contracts; there is no independent second consumer.
Future expansion should start from labelled examples of specific missing rules, not a language or
package-count target. This completes the requested expansion assessment without expanding scope.

## Admission regression boundaries

Review of the initial integration at `9907fa47` found the following gaps. Retain these independent
regression scenarios alongside the PRD mapping above:

| Finding | PRD items | Repair and regression |
| --- | --- | --- |
| Anonymous missing delimiters were not visited, so `function f() { return 1;` could be admitted | 7, 16, 24 | Traverse all syntax children for error/missing nodes; keep behavior analysis on named nodes |
| Escaped static import/export specifiers could silently disappear; pure type exports could spuriously analyze runtime dependencies | 6, 22, 25 | Explicit unsupported-dependency gaps; skip only provably type-only module clauses, including inline type specifiers |
| `delete` separated from its operand by comments/newlines was treated as an environment read | 19 | Recognize the grammar token rather than a source-text prefix |
| Implicit teardown swallowed command cancellation and uninstall removed files before the outer error check | 26 | Propagate terminal failures through artifact and teardown orchestration before file/receipt removal; ordinary teardown failure policy is unchanged |
| Native Windows script separators differed between snapshot lookup, analysis identity and launcher | 12, 16, 20 | Use existing logical-path conventions for planner references and normalize native Windows launcher/entry names |
| Lint lost diagnostics from completed entries when a later entry in the same category exhausted the budget | 26 | Recover completed safe scope diagnostics from the command session, retaining incomplete status and terminal failure |

These repairs introduced rule version 2, invalidating older comparison identities without changing
the report schema. Regressions reproduce malformed-source admission and post-cancellation file
deletion before repair. Native separator and cumulative-budget tests exercise the production
planning/evaluation and lint paths. The fixes restore the already documented public contracts;
no new permission, bypass option, confirmation or runtime sandbox is introduced.

Readiness fixtures exercise positive/negative examples across three simulated platforms and both
trust states, normal Plan approval, non-blocking gaps and observation-only authoring. No-Plan
Frontend evaluation rejects before invocation writes or process starts. Real CLI regressions cover
evaluation/lifecycle entry points with declaration differences and malformed source, including
persistent trust and `--yes`; receipts and destination content stay unchanged.

## Platform evidence and limits

Historical full-workspace native CI checkpoints cover Windows x86_64, Linux x86_64 and macOS arm64:
[initial admission repairs at `39edf006`](https://github.com/biulight/shine/actions/runs/39713599387),
[rule-version-6 repairs at `aa9b81f8`](https://github.com/biulight/shine/actions/runs/39962137705), and
[presentation repairs at `5da9fa46`](https://github.com/biulight/shine/actions/runs/39976619157).
The separate [Rust 1.90 check](https://github.com/biulight/shine/actions/runs/39713599318) establishes
MSRV compilation at the initial checkpoint. These records apply only to their recorded commits;
simulated platforms and cross-target compilation do not establish native execution. Static analysis
and these regressions do not establish exhaustive script-effect coverage or hardware authorization.
