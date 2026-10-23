# Preset code analysis implementation

Status: first-release implementation and acceptance complete. Stages A–D are implemented; stage E
scope assessment is complete. Core, Frontend, CLI and native checks are supplemented by successful
Windows/Linux/macOS CI, Rust 1.90 MSRV and documentation validation at `39edf006` (2026-10-22).
Contract: [ADR 0150](../decisions/0150-preset-code-analysis-admission.md).

## Source rules, version 2

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
| Network | Unshadowed `fetch` and supported immutable aliases, literal HTTP(S) URL; URL-normalized host compared with exact host or `any` | Dynamic/escaped URL literals are gaps; credentials, URL path/query/fragment never appear in reports |
| Node files | `readFile[Sync]`, `createReadStream`, `writeFile[Sync]`, `appendFile[Sync]`, `createWriteStream`, `unlink[Sync]`, `rm[Sync]`, `rmdir[Sync]`; supported `fs.promises` forms | Literal paths only; exact existing logical permission identities, no invented globs/inheritance; ambiguous native path spellings are gaps |
| Bun files | `Bun.write`; direct `Bun.file(literal).text/json/arrayBuffer/bytes/stream/exists/delete/writer` | Constructing a BunFile alone is not a read; indirect BunFile aliases and dynamic paths are gaps |
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

Uncorroborated declarations do not block. They also do not remove gaps. Known command identity
plus a subprocess gap remains incomplete even if file/network declarations are supplied.
Only difference and failure diagnostics block, and all diagnostic classes can coexist.

## Resource limits

Defaults are 1 MiB per source; 256 files and depth 32 per dependency graph; 16 MiB and 2,048 unique
source files per command; 50,000 accumulated facts/comparison diagnostics, 2,000,000 syntax nodes
and 100,000 parser progress callbacks. Byte, file and parser work are charged on cache misses;
per-entry diagnostic work is still charged on cache hits. These are conservative work limits,
not a time/memory guarantee or a claim of exhaustive analysis. Binding lookup is indexed by name;
more than 128 candidates for a name is unresolved rather than allowing unbounded scope matching.

Local file/graph limits and syntax errors produce blocking reports that other independent entries
can continue past. Cancellation and cumulative limits latch a terminal failure on the session,
including for later cache hits. Parser interruption and iterative node traversal check that state.
Core orchestration must propagate it before scheduling more source work or script processes.

## Evidence so far

- Six real production entry files in Surge and Clash Verge parse successfully. Largest entry:
  13,979 bytes; largest reached graph: two files. Repeated analysis is identical and adds no parses.
- Without entry-specific env mapping, these real samples yield zero definite differences and
  4–213 gap locations per entry. Most targets are dynamic and many calls are user-defined;
  these numbers are diagnostic inventory, not a security/coverage percentage or proof that their
  declarations match. Runtime integration must supply each actual environment contract.
- Manually labelled rule fixtures cover literal host/path/command differences, import aliases,
  shadowing, source-to-target mappings, env writes versus reads, unknown subprocess effects,
  dependency cycles/missing/escape/ambiguity, parse failures, privacy and cumulative limits.
- A real Rust 1.90 `cargo check --locked --workspace` passed on macOS after the MSRV update.
  Native Windows/Linux parser builds also passed with Rust 1.96; those runs do not replace the
  separate Rust 1.90 MSRV check.

Do not describe the bounded analysis as exhaustive enforcement of script behavior. Full acceptance
requires the consumer-boundary and native-platform checks as well as the pure analyzer tests.

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

Plan/approval/review schema 3 binds a compact analysis identity plus safe scope reports. Author Plan
schema 3 omits that binding identity, lint report schema 2 reports analyzed entry labels, fixture
report schema 2 adds analysis diagnostic assertions and explicit unfinished cases. Fixture input
schema 1 remains readable. Generated schema reference version 4 names the new report schemas.
Author Plan exit success still describes valid hypothetical planning; `ready` expresses admission.
Lint differences/failures are errors, gaps are warnings, and fixture negative assertions may pass
only for completed cases. A terminal analysis failure cannot be absorbed as an expected blocker.

Initial integration evidence: eight new Core/author tests, 227 analysis/planner/approval/frontend/
author-tool tests and 48 CLI review/author-rendering tests passed. Execution admission checks added
after this checkpoint and the complete acceptance evidence are described below.

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

229 focused runtime/planner regressions passed after admission was added. Additional tests passed
for dropped preparation, child interruption, graceful real-process cancellation, and forced task
abort: forced abort may retain the old invocation, while the next operation creates a different
invocation and never executes from the retained directory. The existing inherited-terminal test
also confirms signal classification and foreground restoration. These tests do not replace the
consumer-boundary and native checks recorded below.

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

Integrated lint of the complete built-in collection finishes with no errors and eight analyzed entry
identities. Surge has five identities and Clash Verge three, counting the typed entry/script pairs
rather than unique source files. The dominant gaps are unknown calls (508 and 192 diagnostics,
respectively, across their lint scope/platform comparisons), followed by dynamic targets and unknown
environment identities. These counts include repeated contexts and are not a coverage percentage.
No definite built-in declaration mismatch was observed with actual entry environment contracts.

Keep the first release limited to App Bun JS/TS and local dependencies. Additional languages would
not resolve the dominant dynamic-target/call gaps and would add parser, binding and platform costs
without corresponding evidence of value. Do not split `shine-analysis` yet: the current consumers
share Core's snapshot, declaration and input contracts; there is no independent second consumer.
Future expansion should start from labelled examples of specific missing rules, not a language or
package-count target. This completes the requested expansion assessment without expanding scope.

## Integrated verification

- Local workspace all-feature regression run: 1,986 tests passed; its sole new-test failure was an
  obsolete test command option, corrected from `--force` to `--replace-managed`. The corrected CLI
  regression and Frontend/admission selection (18 tests), plus subsequent acceptance additions,
  passed separately. All configured commit hooks passed, including the complete current suite.
- Native Windows: 34 analysis/Core admission tests and the real CLI blocking regression passed.
  Native Linux: 35 tests (including Unix process cancel/abort cleanup) and the same CLI regression
  passed. Both used Rust 1.96 and an isolated uploaded source archive with SHA-256
  `1b885b9b06c17cdccdc00764e31da19c4e58f888af09cb3d012abe9f606d9912`.
  Later additions changed tests/documentation only. Windows emitted an existing unused
  `sudo_command` warning; no production install or user preset activation was performed.
- Current production sources passed locked Rust 1.90 workspace compilation and all-target,
  all-feature Clippy with warnings denied. Built-in collection lint completed without errors.
- English/Chinese locale parity, website typecheck and both locale builds passed; generated next
  manual pages contain the new code-check anchors. Skill frontmatter and local links were checked.
- Sandbox-only Unix socket failures were rerun successfully with socket access. Initial pnpm
  tool-cache lock write failure was resolved by allowing the pinned package-manager cache setup.

## Follow-up review against the PRD (2026-10-22)

Review baseline: `9907fa47`, including the three implementation commits. The second pass rechecked
all 27 acceptance items across analyzer, typed trigger selection, private evidence, locked approval,
explicit evaluation, author reports and native platform paths. It found gaps in the first pass:

| Finding | PRD items | Repair and regression |
| --- | --- | --- |
| Anonymous missing delimiters were not visited, so `function f() { return 1;` could be admitted | 7, 16, 24 | Traverse all syntax children for error/missing nodes; keep behavior analysis on named nodes |
| Escaped static import/export specifiers could silently disappear; pure type exports could spuriously analyze runtime dependencies | 6, 22, 25 | Explicit unsupported-dependency gaps; skip only provably type-only module clauses, including inline type specifiers |
| `delete` separated from its operand by comments/newlines was treated as an environment read | 19 | Recognize the grammar token rather than a source-text prefix |
| Implicit teardown swallowed command cancellation and uninstall removed files before the outer error check | 26 | Propagate terminal failures through artifact and teardown orchestration before file/receipt removal; ordinary teardown failure policy is unchanged |
| Native Windows script separators differed between snapshot lookup, analysis identity and launcher | 12, 16, 20 | Use existing logical-path conventions for planner references and normalize native Windows launcher/entry names |
| Lint lost diagnostics from completed entries when a later entry in the same category exhausted the budget | 26 | Recover completed safe scope diagnostics from the command session, retaining incomplete status and terminal failure |

Rule version is now 2, invalidating comparison identities under the old rules without changing the
report schema. New regressions reproduce the malformed-source admission and post-cancellation
file deletion before repair. Native separator and cumulative-budget tests exercise the production
planning/evaluation and lint paths. The fixes restore the already documented public contracts;
no new permission, bypass option, confirmation or runtime sandbox is introduced.

Final follow-up verification: all 1,996 workspace all-feature tests passed (two intentionally
skipped). Two tests were marked non-failing `LEAK` under parallel load; both passed serially without
that marker (`install_dry_run_uses_generator_fallback_without_executing_code` and
`overlay_permission_error_identifies_cause_and_global_config`). Windows passed 41 analysis/admission
tests plus the interrupted-teardown test; Linux passed 42 plus that test. Final Clippy, locked
Rust 1.90 workspace compilation, formatting and typos passed. Built-in lint remains complete with
zero errors. Remote task-owned validation copies were removed after verification.

The uploaded review archive SHA-256 was
`2bee7ef5eb8eae3ebf24fcf00c4e4016abe6b3a986b2b9c5fac6101b02316824`.
Final inline-type refinements were then synchronized and rerun on both instances:
`analysis/syntax.rs` SHA-256 `95f36f7963b3f2d9350ba5a702bc7f380c8ade1084cb5e7fa7fc0dc1864e2f63`,
`analysis/tests.rs` SHA-256 `0e45eb3d5dec4253ba7ad49c96e83cc7421e98a269a296c70390e627ac6c5850`.
The follow-up was committed as `39edf006` and pushed to `release`; final CI evidence follows.

## Final release-branch CI acceptance (2026-10-22)

Verified remote `release` SHA: `39edf006a41ef75a31305f48ab4e195b73446cbe`.
All runs below target that exact commit and finished with `completed / success`; no CI repair or
rerun was needed. Stage C–D integration is recorded in `9907fa47`, followed by the review fixes in
`39edf006`. This closes the first-release implementation and acceptance record for all 27 PRD
scenarios indexed above. Stage E remains an assessment to retain the initial scope, not an
implementation of additional languages or a separate package.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,733 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39713599387/job/119610405660) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 1,994 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39713599387/job/119610405851) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 1,996 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39713599387/job/119610405894) |
| Quality | Bun TypeScript checks, formatting, compilation, preset authoring examples, Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39713599387/job/119610405849) |
| MSRV | Rust 1.90.0 `cargo check --locked --workspace` passed | [MSRV run](https://github.com/biulight/shine/actions/runs/39713599318) |
| Documentation | Typecheck, version checks, locale parity and all-locale builds passed; Pages and server deployment succeeded | [Documentation run](https://github.com/biulight/shine/actions/runs/39713599319) |

The [CI workflow](https://github.com/biulight/shine/actions/runs/39713599387), MSRV and documentation
workflows all completed successfully. These results apply to the tested commit and complement the
local and isolated native evidence above; they do not claim exhaustive script-effect coverage or
constitute a tagged product release.
