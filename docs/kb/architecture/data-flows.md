# Data Flows

End-to-end flows that span multiple modules and are not visible in any single file. For module
ownership and the per-command routing table, see [`module-map.md`](module-map.md) — this file only
records the cross-module sequences and their gotchas.

## Workspace secret file groups

Version 3 workspace + explicit groups + mode → root-constrained mappings → metadata/Git preflight
→ shared path locks → captured workspace and file bytes → existing tagged binary crypto.
Overlap checks and lock hashes share Unicode canonical-decomposition and case-folded path keys;
this includes the workspace definition and absent mapping paths.
Git preflight reads the complete repository index as NUL-delimited paths and compares the same
case/Unicode keys against the repository-relative target, even from a nested working directory
or when the tracked working file has been deleted.
Sealing prepares all ciphertext, rechecks snapshots and writes sources atomically per file;
optional plaintext removal rereads/decrypts all written sources and verifies snapshots before
deletion starts. There is no multi-file transaction or automatic build writeback.

Run decrypts all selected sources, compiles ordinary workspace env when present, rechecks inputs,
and creates private absent targets with an ownership guard. It supervises the child process group
or Windows Job Object, cleans owned files by handle identity and owned empty directories, drops locks
and captured inputs, then propagates the child status. Preparation cancellation drops guards.
Existing/replaced targets are never automatically adopted. Metadata-only dry-run stops before
password tools and writes. Cleanup compares metadata-only path identity with the held ownership
handle, so Unix read-permission changes do not strand an unchanged owned file. A child-deleted
target is already cleaned after boundary checks; continue cleaning other mappings and retain the
child status. Replacement files/directories/links and non-absence errors still fail cleanup. Binary age decrypt
inherits stderr only for required phone QR or explicitly requested phone messages; ordinary captured
diagnostics are discarded. Binary encryption owns the child through concurrent pipe I/O and wait,
terminating and reaping on pipe failure or output overflow.
Persistent export uses the same selection, preflight, shared locks and captured binary decryption,
without compiling env sources or launching a child. After all decryption and snapshot checks succeed,
exclusive private creation writes/syncs each absent target under an owned-object rollback guard.
Yield between writes so controlled cancellation drops the guard. Recheck inputs throughout writes
and target identities before relinquishing cleanup responsibility. Success retains files/directories;
errors attempt rollback and report possible residue without deleting replacements. Existing targets
remain a conflict for later export/run. There is no cross-file transaction or crash recovery.
See [ADR 0112](../decisions/0112-persistent-workspace-secret-file-export.md).
See [ADR 0110](../decisions/0110-workspace-secret-file-groups.md).

## Pure security Plan assessment

`shine-core::plan` defines the Phase 3 approval contract used by protected mutation.
`runtime::planner` consumes one immutable `PresetSnapshot`, a validated App/Shell/managed Sys,
exact Sys bootstrap, App refresh/artifact, or Sys profile request, captured runtime inputs,
manifests, receipts, and live resource observations. It emits
ordered semantic steps plus derived permissions, optional author capability statements, and typed
code boundaries. Missing environment/elevation contracts, uncomputable structured requirements, or
a blocked step make the Plan non-ready. Planning cannot invoke host mutation or
Preset code, and its output carries no content, env values, secret plaintext, raw errors, or raw
command arguments.

The effective Preset snapshot hashes sorted logical paths, bytes, captured executable flags, and
trust layers without hashing its physical checkout root. Target selection from immutable request/Preset input occurs before
host-state reads. Filesystem and split-DNS observation traits expose manifests, receipts, live
resources, launchers, and system state without exposing write/process/privileged/apply methods.
Planners hash every outcome-affecting observation using the same framed SHA-256 builder. Plain env
values contribute hashes; secrets contribute only caller-supplied opaque handles or versions.

```text
validated target + immutable Preset snapshot
    → observation-only manifest/receipt/live-state capture
    → ownership and lifecycle assessment
    → derive typed effects + retain optional author statements
    → classify triggered unisolated code and evaluate target trust
    → ordered payload-free PlanV1 + state/Preset digests
```

Generator, hook, artifact, Shell delivery, bootstrap, and profile-code triggers are modeled as
unisolated code boundaries plus conservative semantic steps. Classification never depends on an
author's `opaque_code` field. App assessments record typed triggered entry kinds; a static filename
beginning with `generator:`, `hook:` or `artifact:` is ordinary data, not a code trigger.
Code is never run during planning and missing or stale target-scoped
trust remains a blocker for external code delivery/execution. A supported receipt can drive uninstall after source
disappearance, but cannot recreate missing teardown code.
Planner and App/Shell/Sys action executors share `review_path` root-aware filesystem identity
encoding. CLI display translates those identities only after exact permission/provenance matching;
the displayed path is never used to grant access or compute an approval.
Typed filesystem operations also record exact presentation-purpose groups. The CLI summarizes
uniquely attributed installation, maintenance and recovery material in ordinary ready Plans;
user destinations, ambiguous entries, blocked Plans and recovery reviews stay explicit. All
Security Plan commands accept `--verbose`; the display preference is scoped to the command future
and never changes the execution request. Full permissions and purpose groups remain fingerprint-bound
(ADR 0094). `lifecycle_plan/render.rs` is a display-only facade over compact, detailed and
bootstrap layouts, permissions, code boundaries and labels. Bootstrap retains its ordered scopes
and distinct permission formatting; incomplete attribution falls back to the full detailed review.
No rendering module creates approval or changes the reviewed Plan (ADR 0126).

CLI review creates approval for one exact ready Plan. Apply deliberately follows:

```text
capture current source/state → regenerate Plan → match approved fingerprint + permissions
    → execute existing Core lifecycle → return LifecycleResultV1
```

App, Shell, managed Sys install/upgrade/uninstall, exact Sys bootstrap, App refresh/artifact, and
Sys profile enable/disable route through this flow. Untargeted `shine upgrade` plans all three
lifecycles, displays only scopes needing review by default, confirms once when there are actions,
and prevalidates all three before protected mutation starts. `lifecycle_plan/prepared.rs` first
checks the complete Shell/App/managed-Sys upgrade batch shape, then captures and validates each
runtime in that same order. Named one-shot domain handoffs replace positional vector extraction;
adapters consume them and receive their domain's typed Core report. The shared managed-Sys adapter
still accepts its existing install/remove paths through a separate managed-Sys handoff. Neither
handoff construction nor prevalidation replaces Core's fresh apply-time Plan/permission gate
([ADR 0123](../decisions/0123-typed-upgrade-handoffs-and-launcher-tests.md)).
The `self_install.rs` facade delegates configuration orchestration to `self_install/config_upgrade.rs`;
Update checks (including explicit generator evaluation), release installation and local binary
publication have separate owners.
Compatibility checks, env upgrade and Shell/App/Sys review/prevalidation/execution retain their
existing order. Target routes normalize App/Shell file/command suffixes to categories, reject malformed
Sys targets and limit stale pruning to App. Selector parsing preserves literal category whitespace
before matching; blank-operand rejection does not trim nonblank identities. The private
`TargetUpgradeSummary` preserves each domain's
counting units, restart hints and authoritative fatal report total; aggregate counters, lazy no-op
headings and App-before-Sys failure precedence remain separate (ADR 0127).
`upgrade --verbose` expands relevant scopes, while `upgrade --verbose --full-plan` displays every planned scope and step. Compact
review combines known routine App/Shell cache writes into one internal maintenance summary;
verbose review retains their category/file steps. Permissions, code boundaries, preservation,
blockers, and unknown diagnostics remain visible (ADR 0099). Ready cache-only App scopes move
uniquely Core-attributed cache write permissions under maintenance and stale-source notices
under Warnings; ambiguous effects retain their original App review.
`upgrade --pull` pulls and reloads first. Existing dry-run/status remain separate preview/inspection paths.
Scoped external-code trust, ownership, and administrator authorization remain additional gates.
App Plan and Action IR construction share static relocation eligibility. Typed optional step kinds
carry relocation, stale-prune, forced-removal and shared Shell code intent; they enter the fingerprint
and replace diagnostic-string routing without turning Plan steps into executable Actions. App apply
methods share exact approval/IR/permission validation and retain action-specific live-state checks.
Recovery planning dispatches read-only action assessments after receipt-conflict checks; approved
recovery re-plans under the lock and dispatches rollback handlers in reverse action order (ADR 0104).
CLI approval orchestration lives in `cli/src/lifecycle_plan.rs`; its `render.rs` child only projects
reviewed Plans into terminal lines. App upgrade phases share one mutable receipt/report state; Shell
transaction orchestration retains locks and durability boundaries while dispatching resource-specific
preparation and recovery handlers (ADR 0111). These extractions do not change Plan fingerprints.

App ownership preflight now returns captured per-file facts or a typed stop decision before the
generated/static planning phases. Shell convergence visits category cache, shared snapshot and
affected siblings before per-command assessment. The same accumulators retain observation and
step ordering; these facts do not survive into approved execution. App install/removal coordinators
retain the existing preparation effects, per-file receipt commits and finalization order, projecting
typed outcomes into unchanged local reports. Sys journal coordination dispatches through a borrowed
Sys-only action view to resource helpers. CLI `lifecycle_plan/classification.rs` centralizes terminal
grouping over the unchanged Plan, with unknown diagnostics remaining visible (ADR 0115).

App multi-file execution commits each resource's matching receipt before proceeding. A later receipt
failure retains that action's journal for explicit recovery while earlier committed resources remain
committed. Replaying the lifecycle approval cannot resume or recover the interrupted operation;
recovery and the remaining lifecycle work each require their own fresh review. The outer lifecycle
lock is acquired before approval revalidation, so state changed while waiting rejects approval before
preparation can reconcile caches or evaluate generators.

After approval, the CLI upgrade report applies the same relevance rule to unchanged App and Sys
outcomes; `--verbose --full-plan` restores their individual rows (ADR 0098). The structured
lifecycle result still includes every selected item.

Sys bootstrap uses the dedicated `sys-bootstrap` Plan operation rather than a lifecycle install.
Interactive or profile selection resolves to an exact ordered item list before planning. The pure
planner observes command/path presence without executing detection, binds run-manifest,
environment/proxy and profile state, and derives package-provider, script, administrator and
profile permissions. It retains item-local permission resolutions plus separate profile and shared
write scopes before merging the aggregate set. The CLI uses these fingerprint-bound scopes for
bootstrap review; `--verbose` expands identities and diagnostic codes (ADR 0086). Approved execution re-plans before any detection command, installer,
materialization, profile write, or receipt mutation. Its existing domain report remains separate
from `LifecycleResultV1`.

Capability statements are parsed from the same immutable snapshot: one App category table, one
table per Shell command/platform variant, and one table per Sys item. Static validation checks
version, placement, structured paths, payload-free identities, and duplicates without executing
Preset code. Typed metadata describes Core-bounded effects; explicit command, filesystem, network,
and system entries remain unverified author statements. Their absence does not block planning.
Environment input identities and Administrator authorization keep executor semantics, so a missing
required contract remains non-ready. Shell/Sys defaults resolve per target before fingerprinting.

Core classifies triggered App, Shell and Sys executable entries independently of permission fields.
The compatible v2 unrestricted identity is an author statement and cannot alter classification.
The `preset` trust target enumerates current requirements and persists separate per-target grants.
Snapshot grants bind the complete effective category files, bytes, executable flags and source layers. An explicit
`--development` grant binds the target/capability and current local source-root/layer
identity while allowing content changes. Schema v1 grants require review again.

## App script execution snapshots

App `generator`, `hooks` and `artifact` modules own their distinct execution contracts. Generator
activation/manual policy gates precede trust and bounded execution; failures redact untrusted stderr.
Hooks stop on a category failure but continue other changed categories, preserving prior notes.
Artifact preview returns before effects, implicit execution captures output and reports non-fatal
failures, and explicit execution inherits I/O and propagates failure. Environment mappings remain
entry-local. These modules share the existing trust gate and `app_script` mechanism, not a generic
runner (ADR 0126).

After trust and Plan approval, App generators, script hooks and artifacts materialize the complete
captured effective category into a fresh owner-only invocation directory below
`shine_dir/runtime/app/<category>/`. Entry paths and cwd use that copy. `SHINE_APP_DIR` and
`SHINE_APP_SOURCE_DIR` identify the effective copy; `SHINE_APP_OVERLAY_DIR` identifies the captured
effective overlay subset. Without a captured overlay, execution explicitly injects an empty
`SHINE_APP_OVERLAY_DIR` so ambient or caller-injected paths cannot select uncaptured files.
Relative imports therefore cannot reopen changed checkout helpers.
Bun dependency mode still comes from the captured script's source layer. Execution and spawn errors
both clean only the invocation tree; persistent artifact output uses the existing state/cache
paths. Plans declare the create/remove effects before execution (ADR 0092).

## Shell availability and ownership inspection

Shell inspection joins the effective platform-selected Preset commands with manifest-only targets.
`shell/inspect.rs` owns the read-only flow: load categories/manifest once, compare each category
snapshot once, observe each command in the existing order, then project changes/status without Host
access. A private target binds captured category/file, matching receipt and category snapshot state;
private observations retain desired delivery and source/launcher evidence for that assessment only.
Available rows stay in category/command order; receipt-only rows follow in manifest order. Preserve
snapshot comparison errors as non-current, source read error contexts, status priority and diagnostic
wording. Inspection and snapshot comparison accept only `FileSystemObservationHost`; they acquire no
operation lock or approval and execute no Preset code. See
[ADR 0120](../decisions/0120-read-only-shell-inspection-phases.md).
Receipt-only records are inspection data and never enter execution as reconstructed metadata.
The planner and inspection share the observation-only launcher probe with category, overlay, and
receipt roots. CLI update separates conflicts/missing Presets from pending updates; detailed views
keep the same status. Upgrade preserves absent-source commands and blocks shared snapshot
replacement when it would affect a retained missing sibling. Explicit uninstall remains
receipt-driven even after an external category disappears (ADR 0081).
Shell upgrade planning also projects source-command receipts in execution order and reuses the
read-only profile-file comparison. Only observed profile differences contribute profile steps and
permissions; the compact review labels them as internal Shell integration (ADR 0096).

The CLI `status/shell.rs` adapter preserves command identity and Core status/attention flags while
projecting existing display fields. List grouping uses typed severity, update outcomes and pending
matching use explicit `shell/<category>/<command>` identities, and diff/uninstall selectors use
`<category>/<command>`. Lifecycle target construction retains the legacy terminal-component
normalization for non-canonical receipt command names. Display labels and symbols never feed those
decisions. Snapshot capture,
read order, category sorting, last-row tie selection and partial-install rules stay unchanged;
this projection does not grant approval or launcher ownership (ADR 0125).

## Frontend Service inventory

The CLI and future adapters construct `FrontendService` from the same captured `CoreRuntime`,
immutable effective Preset snapshot, and host observation ports. Inventory parses App/Shell/Sys
metadata through Core, loads version-gated manifests and receipts, and observes target-launcher
presence without executing Preset code, detection commands, or host mutations. Ownership and
launcher conflicts remain part of later inspection contracts.

```text
captured RuntimeContext + immutable PresetSnapshot + observation host
    → Core metadata + manifest/receipt/launcher evidence
    → canonical available/installed target union
    → InventoryReportV1 + safe diagnostic codes
    → adapter-only grouping, filtering, and rendering
```

The stable report never contains the manifest path, destination, content, argv, environment value,
secret, or local source error. Manifest-only targets receive `frontend_inventory_preset_missing`;
the CLI compatibility adapter still applies its released App/Shell visibility rules. Existing
`RuntimeEvent` values and inspection paths remain private side channels and are not serialized.

Plan review remains separate from approval: future AI adapters may return a review request, while
only trusted human-facing frontends create a one-shot approval after affirmative review. Apply
continues to recapture inputs and validate a regenerated exact Plan.

## Frontend inspection and review

`frontend/inspection.rs` projects Core App/Shell/Sys assessment into `InspectionReportV1` while
returning local-only domain details for CLI rendering. Physical paths, raw errors and diff contents
never enter the report. Opaque resource identities hash normalized logical source names. App
update applicability and update lifecycle outcomes are service-owned; manual generator changes
remain refresh-only. Sys inspection additionally includes bootstrap receipts as recorded state,
without changing inventory v1's managed-only installed Sys compatibility contract.
CLI verbose status retains the same inspection's managed installed rows, updates and safe states;
current and missing-source records remain visible without another query or live probe.

Default App/Shell/Sys service inspection and `ReadOnlyFrontend` inspection accept only a
`FileSystemObservationHost`. App default inspection uses the existing selected static assessment;
trusted explicit generator evaluation feeds the same report projection through its separate
execution-capable entry. `runtime/sys/inspect.rs` reads managed receipts and compares desired
snapshot inputs without live resource/provider probes. Preset parsing and desired Sys builders
need no Host; manifest version/error rules and read order remain unchanged (ADR 0129).

The generic runtime harness now enters these Frontend inspection paths: App assertions use a
receipt and actual destination, and Shell assertions use declared command availability. Both
check read-only Host operations. The retired generic `inspect_snapshot` path and its DTOs have
no production role. CLI lifecycle tests call dispatch's `*_approved` handlers with an explicit
confirmation choice; removing convenience wrappers does not change approval flow (ADR 0138).
The observation-only App capability test and static/generated partial-failure regression also
enter `FrontendService::inspect_apps`; the old observer-ignoring runtime forwarding method is
removed. Read-only assessment, explicit generator evaluation and report projection remain the
production owners (ADR 0139).

Shell source discovery and description tests follow captured embedded/external snapshots into
`CoreRuntime::shell_categories`; the retired CLI `.sh` scanners are not compatibility owners.
Validation output tests follow `validate_path` into Core's discovery, validation and report
aggregation, then render the returned report. Golden normalization touches only already-asserted
temporary paths, never validity, diagnostics or counts (ADR 0135).

Asset copy tests enter `copy_embedded_preset` and pristine `extract_embedded_prefix`; concurrent
export tests enter `handle_preset_export` and each Config's `PresetAssetReader::extract_all`.
Both retain the same shared extraction/publication checks. Removal tests instead enter approved
Shell uninstall and Core's receipt-aware lifecycle, never asset-reader deletion. Config file
regressions use `read_minimal_config_file`; global-only initialization uses the production
`load_global_runtime(true)` owner (ADR 0136).

The CLI list facade delegates ordered observations to `list/query.rs`, pure grouping and action
selection to `list/projection.rs`, and output to `list/render.rs`. Local query results retain App
inspections for diff collection without re-evaluation. Simple Shell inventory projects only
available/installed targets; a receipt can establish installation even when its launcher is missing.
Managed Sys inventory uses the frontend's OS/managed receipt filter; its former test-only runtime
query has been removed (ADR 0134). Update retains two Shell captures followed
by App, Sys and missing-source queries before output; status prints its initial note and Shell
attention before loading App/Sys, then queries missing App sources through the existing read-only
frontend. Receipt-only App resources prevent an empty-status result and reuse the retained-content
notice/uninstall preview; they do not enter active App rows or summary counts. Detailed diff
collection remains after the original empty-result checks (and after initial update attention
output). Renderers still perform the existing per-row
diff display reads. No shared snapshot optimization, new evaluation, approval or mutation is added.

`frontend/review.rs` dispatches typed workspace-local `ReviewRequest` values to existing pure
planners and wraps the unchanged `PlanV1`. CLI review and preparation use this service. Neither
request nor report contains approval; opaque input versions remain attached to the reviewed domain
request through execution. See ADR 0078.

## Frontend operation state and safe events

Each domain executor loads and validates its journal once, extracts recorded action progress, and
passes that exact journal and bytes to its existing recovery planner. The service hashes the local
operation identity and emits counts plus the payload-free recovery Plan. An idle report means no
journal was observed; journal presence does not establish that a process is alive. Counts are
durable bookkeeping, not a substitute for receipt/positive-marker and live-resource validation.
Corrupt or future journals return a safe diagnostic without cleanup.

`frontend/events.rs` explicitly projects each `RuntimeEvent` variant to a versioned event kind,
execution-local sequence, optional reviewed canonical target and typed outcome. It never copies
raw codes, details, output, labels, paths or environment values. `ProjectedObserver` forwards the
original event only to trusted local presentation while sending the safe projection to the frontend
sink. Events convey progress, not approval, a durable replay cursor or recovery authority. See
ADR 0079.

## Frontend authority and execution

`FrontendService::capture` accepts distribution-resolved context and source settings and delegates
effective snapshot capture to the shared host-backed bootstrap. Trusted distribution code supplies
an opaque configuration revision. `ReadOnlyFrontend` returns only safe reports and safe diagnostic
errors; it has no execution, generator evaluation, runtime or approval constructor access.

Trusted human review retains the exact request and configuration revision alongside the Plan.
After confirmation, its consumed `ApprovedOperation` carries that request through fresh validation
and the shared execution dispatcher. CLI preparation recaptures configuration and Presets before
execution; all domain adapters call `lifecycle_plan::execute_reviewed` and only render returned local
details. They do not construct legacy approval, match fingerprints or rebuild execution requests.

The service emits safe progress around all dispatched calls and preserves raw observer events for
local renderers. Normal operations reuse `LifecycleResultV1`; specialized operations and recovery
retain distinct identities in `ExecutionReportV1`. Successful call completion does not imply every
item succeeded. Validation rejection produces no execution events or effects. See ADR 0080.

## Preset source compatibility and migration

`shine preset migrate` is routed before mutable config initialization. Default scope uses read-only
discovery for the active external source and overlay; an explicit repository, category, or manifest
path bypasses active-source selection. Core plans only `shine.toml` changes from one effective
snapshot and returns a content-free report plus process-local candidate bytes. The CLI facade
retains approval and execution order; `preset_migration/discovery.rs` returns named immutable
inputs, `assessment.rs` validates candidates, `render.rs` formats reports/diffs, and `apply.rs`
owns source checks, backup and edits. Source checks run before backup and again before applying
any edit; each file is also checked immediately before its replacement/removal (ADR 0124).

```text
read-only source discovery or explicit PATH → immutable effective/base/current-embedded snapshots
    → released-1.x fingerprint and structural compatibility assessment
    → process-local TOML candidates → authoritative same-scope validation
    → versioned hash/action/diagnostic report + CLI-only unified diffs
    → default-No approval → source hash recheck → complete private backup set
    → per-file hash recheck → atomic replace/remove → final status
```

Only exact old built-in metadata with matching executable identity, safe App schema/hook cleanup,
and overlays that reveal valid lower metadata can become edits. Opaque permissions and Sys v1
dispatchers remain blockers. Managed Git overlays stop at diagnosis. Partial independent edits may
land after the complete backup succeeds, but any remaining blocker or apply failure keeps the final
result non-successful and the retained backup supports manual recovery.

`update` prints this same compatibility assessment while continuing its read-only configuration and
release checks. `upgrade` performs it after an optional pull/reload and before lifecycle planning or
mutation. Neither path applies candidates or creates migration backups.

## Static Preset validation

`validation/source.rs` accepts repository, category or `shine.toml` inputs through an observation-only
Host and captures one immutable external scope using the existing traversal/filtering rules.
`validation.rs` keeps the V1 report and diagnostic aggregation. Permission diagnostics run first,
then all App destination branches, then metadata checks in macOS/Linux/Windows order; the first
platform error ends that category's metadata checks. `permissions`, `destinations` and `sys` own
these separate rules. Sys references resolve only against captured bytes. Authoring, lint and
migration reuse the same scope/report path; the split grants no mutation or process capability
(ADR 0126).

## Preset authoring Plan report

`shine preset plan` is routed before runtime config initialization and background update checks.
Core resolves exactly one category directory or manifest, captures one immutable external snapshot,
and runs static validation against that snapshot. Only a valid category proceeds to planning.

```text
category path + explicit platform
    → one immutable external Preset snapshot
    → same-snapshot all-platform static validation
    → deterministic empty RuntimeContext + InMemoryHost
    → existing App/Shell/managed-Sys/bootstrap planner
    → authoring report (assumptions + steps + permissions + blockers)
```

App and Shell use a first-install lifecycle request. The authoring context explicitly selects Zsh
for macOS/Linux and PowerShell with synthetic Windows profile paths for Windows; it never inherits
the compiling host's shell. Missing Shell template inputs become a typed, redacted blocked step,
not an authoring construction failure. Sys partitions the validated manifest into managed and init
items, then emits separate managed-install and bootstrap sections. The synthetic
context contains no env values, secret versions, trust grants, detected commands, manifests,
destinations, or administrator state, so related blockers remain visible. The output deliberately
drops source/state digests and fingerprints: it is not a security Plan approval and cannot enter an
apply path. Planning may perform only in-memory filesystem or split-DNS observation and never
process, network, privilege, or mutation operations.

## Preset lint

`shine preset lint` shares source-scope capture and all-platform validation with `preset validate`.
Only a validation-clean immutable snapshot reaches lint policy. Core loads the already-authoritative
App, Shell, and Sys models for each relevant platform, deduplicates logical findings, and emits a
versioned report with no physical checkout or suspected private path.
Unavailable App models are a normal per-platform omission. Synthetic contexts select PowerShell
on Windows and Zsh on Unix so lint covers the same resources as validation.

```text
repository/category/manifest path → immutable scope → all-platform validation
    → Core metadata models → quality/portability/minimization rules
    → stable logical diagnostics → optional --deny-warnings exit policy in CLI
```

Default lint success means the Preset is valid even when advisory warnings exist. Strict CI is an
explicit frontend exit policy and does not change report contents or runtime acceptance.

## Preset fixture tests

`shine preset test` loads one category and its versioned `shine.test.toml` from the same immutable
snapshot. Each unique named case selects a platform, materializes only declared observations into a
fresh `InMemoryHost` and isolated context, derives requested trust grants from the exact current code
requirements, invokes the synthetic authoring Plan flow, and compares only declared structured
expectations. Missing expectation fields are ignored; explicit lists compare as sorted sets. Opaque
secret versions feed `PlanningInputVersions`; their text and all synthetic contents stay out of the
report. Fixture parsing exposes no executable or real-host setup path.

```text
category + shine.test.toml → strict fixture schema → platform + bounded host observations
    → fresh InMemoryHost/context + exact derived trust grants → synthetic authoring report
    → structured action/permission/diagnostic expectation comparison
    → stable per-case failure codes + versioned aggregate report
```

## Preset schema reference

`shine preset schema` combines Core-generated JSON Schemas with help rendered from the current Clap
command tree. The CLI adds presentation metadata only; it does not maintain another schema model.

```text
shipped Rust authoring types → schemars draft 2020-12 documents
live Clap preset subcommands → generated long help
    → versioned deterministic reference JSON or compact text index
```

## Preset copy and export

The CLI resolves the selected output root, walks each nested parent without accepting links or
Windows reparse points, and rejects linked/special destination entries. It writes and syncs a
same-directory staging file with final permissions, rechecks the path, then uses Core persistence
to replace an entry or publish without replacement. A concurrent non-force winner is skipped.
Forced replacement does not modify another name pointing at the old inode.

## Preset pack

Packing validates one immutable category snapshot, then separately walks the physical category with
an observation-only host so ignored trees and symlinks cannot evade policy. After policy checks,
Core sorts logical files, excludes the author-only fixture, builds the versioned hash/mode manifest,
and encodes fixed-metadata tar.gz bytes. The CLI resolves the category and output parent through
symlinks and parent components, rejects output inside the category, and performs the requested
atomic output write at the resolved parent. Missing output directories are created only after this
check. Without `--force`, Core persistence publishes the synced temporary file without replacement
(Unix hard-link publication, Windows write-through move without the replacement flag), so concurrent
writers cannot overwrite the winner. Existing entries report `output_exists`, including dangling
symlinks; `--force` retains atomic directory-entry replacement.

```text
category → immutable validation → physical policy scan
    → sorted logical files - shine.test.toml → shine.bundle.json
    → fixed tar/gzip metadata → bytes + SHA-256 → explicit CLI output
```

## Scoped external-code trust

`core::runtime::trust` derives requirements only from the immutable logical Preset snapshot. Each
requirement binds a canonical App/Shell/Sys target, capability kind, digest of the complete effective
category snapshot and source layers, and the target/capability identity.
`core::trust::evaluate_trust`
matches that requirement against versioned grants without consulting project configuration.

```text
complete effective category snapshot + trust layers + target/capability
    → TrustRequirementV1
    → exact match against global owner-only trust.toml
    → trusted or stable missing/stale decision
    → decision bound into Plan state → separate one-shot Plan approval
```

The CLI loads `~/.shine/trust.toml` before constructing `RuntimeContext`. `shine trust grant`
derives and renders the current requirement, confirms with default No, then atomically stores only
the reviewed identities. Category content or layer changes do not match snapshot grants; source
root/layer changes do not match development grants. Old v1 grants remain listable
and revocable but are unsupported for execution. Legacy coarse booleans are diagnostic-only.

## Shell install and uninstall

Shell lifecycle targets are either a category (`utils`) or one command in a category
(`utils/shine-env-export`). Embedded sources and external snapshots remain category-scoped shared
deployment material so a command can consume sibling resources, while launchers and
`shell-manifest.toml` receipts are command-scoped.

Approved lifecycle execution takes `shell-lifecycle.lock` before regenerating and validating its
Plan and retains it through receipt persistence and cleanup. Recovery, completion updates and live
rendering use the same lock; any privileged transaction lock is acquired afterward. Standalone
receipt updates also lock, while internal updates reuse the outer guard (ADR 0106).

`shell/lifecycle` carries selected targets through typed preparation and application states, then
produces pure reports. Preparation preserves legacy filesystem effects; it is not a read-only Plan.
The split retains the original action, manifest, commit-marker and cleanup order. Manifest launcher
reconstruction, Action IR validation and recovery conversion share the borrowed delivery rules in
`action/shell_receipt`; persisted receipt fields and schema versions remain unchanged (ADR 0114).

Inspection, planning, launcher preparation and manifest construction share pure source layout and
`DesiredShellDelivery` facts. No Host observation is retained across human approval. Delivery is
built at the original Bun-policy assessment point. Native PowerShell and live Bun launcher format
identities both participate in receipt current-ness, so a refreshed native launcher converges on
subsequent upgrade review (ADR 0116).
`launcher.rs` groups borrowed invocation fields and expands every native launcher resource through
one preparation function. `launcher/templates.rs` owns pure platform bytes and quoting; it does not
observe or mutate files. Creation and current-status reconstruction retain their distinct legacy
name/format choices. Format markers, companion ownership and exact mode/byte checks remain unchanged
(ADR 0128).

Transaction preparation retains each journal Action and its execution payload in one ordered
`PreparedShellStep`. Apply projects the Action IR from those records, validates it and its exact
permissions, publishes the journal, then executes the same records with the existing per-action
progress saves. Payload bytes stay in memory; journal and recovery schemas are unchanged (ADR 0117).

Journal admission, marker eligibility, commit and recovery dispatch share a private borrowed
`ShellAction` view with exhaustive domain conversion. Commit checks all live resources and receipts
before starting ordered rollback cleanup, then removes the journal only after every cleanup succeeds.
Recovery retains forward receipt projection and reverse resource dispatch through the existing
handlers. Launcher creation/update still commit from matching receipts without positive markers;
other Shell actions keep their explicit markers. No persisted schema or Plan changes (ADR 0118).

Command install filters metadata before transforms and launcher creation, then upserts only the
selected manifest target. Category install retains the existing replace-category reconciliation.
For embedded sources, Core derives one payload-free `ReplaceShellCache` action per selected category
that has actual extraction writes. The action includes missing files and differing existing files
during upgrade or under `--replace-managed`; skipped and unrelated files stay outside the action.
Each changed file binds old/new hash and mode plus a canonical same-directory rollback, while the
category action binds selected command receipt transitions and a positive commit marker. Recovery
projects the old receipt boundary before dependent rendered/launcher recovery and then reverses only
exact created or replaced cache files. After commit it keeps desired files and cleans exact rollback.
For an external snapshot-mode selection with no rendered command output, a changed category tree
derives one payload-free `ReplaceShellSnapshot` action before launcher actions. The journal binds
sorted old/new tree identities, deterministic stage/rollback directories, and selected command
receipt transitions. After saving the desired receipts, Core records a positive commit marker
before cleaning the exact old tree. Before that marker, recovery projects the previous receipts
into both planning and execution so dependent launcher actions are assessed at the same old
boundary, then restores the exact old category tree. Modified tree state blocks the whole recovery.
For every selected command whose lifecycle transforms produce missing or changed output, Core also
derives a payload-free `ReplaceShellRenderedFile` action before launcher actions. One file-scoped
action binds its previous/desired hash and mode, canonical same-directory rollback, all consuming
command receipt transitions, and a positive commit marker. Recovery projects uncommitted rendered
receipt transitions back before assessing launchers, then removes an exact new file or restores the
exact previous file. Once marked committed, it keeps the desired file and cleans only exact
rollback. Execution-time live rendering remains outside this lifecycle journal.
See [ADR 0064](../decisions/0064-transactional-external-shell-snapshots.md),
[ADR 0065](../decisions/0065-transactional-shell-rendered-files.md), and
[ADR 0066](../decisions/0066-transactional-embedded-shell-cache.md) for the distinct shared-source
ownership boundaries.
For a command with no receipt and entirely absent launcher resources, Core derives a payload-free
`CreateShellLauncher` action, writes `shell-operation-journal.toml`, creates the Unix symlink,
Unix generated launcher, or Windows shim pair, persists the exact command receipt, and only then
clears the journal. Profile reconciliation starts after that commit. An interruption blocks later
Shell lifecycle Plans until `shine shell recover` reviews current receipt and per-resource state;
recovery removes only unchanged transaction-created resources or preserves an already receipted
launcher. Install or upgrade may also derive `UpdateShellLauncher` when the command's old receipt
and every reconstructed launcher resource remain exact. Core journals both receipts, moves each
changed old resource to its same-directory `.shine.rollback`, writes the replacement, persists the
new receipt, removes only exact rollback material, and clears the journal. Before receipt commit,
explicit recovery restores exact old resources; after commit it preserves the replacement and
cleans only exact rollback material. The Plan observes and grants every platform resource, including
both Windows shim files. Approved uninstall similarly derives `RemoveShellLauncher` only when the
old receipt and every reconstructed resource remain exact. Core journals, moves each launcher to
same-directory rollback, removes the command receipt, records a positive `receipt-committed` marker,
then cleans exact rollback material. Before that marker, recovery restores the old receipt if
needed and moves exact resources back; after it, recovery preserves the completed uninstall and
cleans only exact rollback. Modified or foreign launchers remain preserved and outside this proof.
When uninstall selects the last command receipts consuming a regular managed rendered path, Core
also derives `RemoveShellRenderedFile`: the exact file moves to same-directory rollback before the
receipt set is removed, and a positive marker separates receipt absence from committed deletion.
Before that marker, recovery reconstructs missing receipts and restores only the exact old file;
afterward it preserves absence and cleans exact rollback. Unselected consumers and unrelated
rendered files are preserved. Invocation-time live rendering takes the same cross-process lock,
refuses a pending journal, and re-reads its receipt before atomically replacing last-known-good
output.
When the final selected receipts release embedded cache files or an external snapshot category,
Core derives `RemoveShellCache` or `RemoveShellSnapshot`. Exact files/trees move to rollback before
receipt removal, and a positive marker distinguishes committed absence from an interruption that
must reconstruct old receipts and restore exact material. Shell profile reconciliation uses
`ReconcileShellProfile`; recovery merges only the recorded Shine sentinel transition into the
current profile so unrelated later edits survive.
Status treats a manifest receipt or a compatible legacy launcher as installed; extracted source
files alone are only cache state. Command uninstall removes only the selected managed launcher,
rendered output, and receipt, rebuilds source-command profile wrappers from the remaining launchers,
and removes shared category material only after the last installed command is gone. Foreign command
entries are never removed.

Every mutating or dry-run Shell lifecycle entry loads `shell-manifest.toml` before extraction,
snapshot, render, launcher, receipt, or profile work. Legacy v0 normalizes in memory, successful
mutations save schema v2 (legacy v0/v1 normalize in memory), read-only status/update does not rewrite it, and a future version fails
before mutation. The Shell adapter emits one `shell/<category>/<command>` outcome per installed or
selected command. Read-only update maps typed row changes to `pending` plus write-preview effects;
foreign launcher ownership is `conflict`, not pending, and upgrade preserves that launcher and its
receipt. Shared cache/snapshot/rendered effects attach to affected commands without turning source
presence into installation evidence.

Shell execution emits CLI-private presentation events instead of writing terminal output. The
terminal renderer owns shared upgrade-section state, while writer-backed recording tests pin
quiet/verbose sections, conflicts, profile hints, and stdout/stderr routing.

Embedded source/cache replacement preparation lives in `shell/cache.rs`; shared rendered-file
preparation lives in `shell/rendered.rs`. Keep cache overwrite/preserve rules, source/mode selection,
transform inference, shared-output conflict checks, report ordering and lifecycle preparation
side effects unchanged. Both pass named local receipt transitions/removals through
`shell_action_executor/receipt_changes.rs` to the existing V1 Action contracts. The conversion
preserves target, previous/desired receipt, order and optional absence; validation, exact approval,
journal publication, receipt commit and recovery remain with their existing owners (ADR 0130).

## Transformed Live Bun invocation

A `live-bun-v2` launcher binds its absolute installation root and calls hidden `__shell-launch`
once. CLI loads global/project/env override configuration once, then constructs the existing captured
Core runtime. `prepare_live_bun_launch` holds the operation lock while refusing a pending Shell journal,
validating one installed receipt and its env/transform/path contract, and rendering it. The description
contains only the script path, dependency enum and env specs. CLI drops the runtime and releases the
lock before encrypted-first declared-value resolution and direct Bun spawn. Stdio/cwd are inherited,
workspace discovery is absent, and the hidden route skips version notifications. Empty declarations
still allow template rendering, but never inject the whole Config env table. Later env/spawn errors
leave successful rendered output in place; shared output remains mutable after preparation.
The Windows PowerShell entry explicitly encodes argv for `ProcessStartInfo` with shell execution
disabled, preserving empty args, literal `--`, quotes and trailing backslashes under PowerShell 5.1.
It starts the PATH-resolved Shine executable with inherited stdio/cwd; no additional shell is spawned.

Missing format fields select the frozen legacy launcher. Schema-2 manifest plus strict Action receipt
fields protect new formats from old readers. Upgrade, uninstall and recovery reconstruct resources
from the captured format/root, including both Windows shims (ADR 0102).

## App install (`shine app install <category>`)

`CoreRuntime` owns the complete App lifecycle from an immutable preset snapshot: metadata and
destination resolution, one-pass assessment, generators, transforms, Copy/JSON merge, relocation,
ownership, hooks, artifacts, embedded cache, manifest persistence, and Contract v1 mapping. The
CLI supplies resolved config plus `rust-embed` bytes. Shared runtime bootstrap discovers any
external/overlay tree through `FileSystemHost`, constructs the immutable snapshot, and submits the
request; the CLI renders typed events/reports. There is no frontend-specific directory walker,
prepared-file path, or CLI fallback executor.

App content assessment uses private variants for ready transformed bytes, not evaluated, trust
required and evaluation failed. Inspection projects these into its existing content/error/diagnostic
fields and typed statuses; missing, partial and user-modified installed-file states retain precedence.
Install preserves a last-known-good file after generator failure, while upgrade reports it as failed;
both retain its receipt and keep generator detail in the existing local error channel (ADR 0117).

Approved App mutations first acquire the configuration-scoped lifecycle lock, then replan and
validate the original approval against current state. The guard spans generators, hooks, file
effects, and all manifest saves. Install excludes files marked Preserve before generator assessment
and reports them as user-modified without changing their destination, backup, or receipt. Other
App mutation entry points, including explicit recovery, share this lock (ADR 0105).

The Core flow is:

1. **Metadata** — `apps/metadata.rs` parses `presets/app/<category>/shine.toml` (category `dest`,
   optional per-`[[files]]` `dest`, `transforms`, `requires_admin`, …). A file destination overrides
   the category root. Exact `macos`/`linux`/`windows` selection (with `unix` as the macOS/Linux
   fallback) filters a targeted category before env loading or embedded extraction; an exact
   destination overrides the fallback. `{ base = "data-dir", path = "..." }` remains structured
   until a `Config` resolves the current user's platform data directory. Duplicate effective
   destinations fail before any writes. Legacy categories without `shine.toml` (git, starship) use
   `apps/annotation.rs` to read a `shine-dest:` comment from the file itself.
2. **Runtime state gate** — `install_core/manifest.rs` loads `app-manifest.toml` before env
   initialization, embedded extraction, generator execution, or destination writes. A missing
   `schema_version` is legacy v0 and normalizes to v1 in memory; an unsupported future version
   fails before lifecycle mutation.
3. **Transforms** — `install_core/transforms/` applies `jsonc-to-json` and/or `template`
   (`@@VAR@@` substitution from the `[env]` config table) in declaration order, in memory, before
   writing.
4. **File ops** — `install_core/file_ops.rs` backs up any pre-existing user file to
   `<name>.shine.bak`, then writes the (transformed) content to `dest`. Destinations with
   `requires_admin = true` (e.g. `/etc/docker/daemon.json`) go through the sudo path, serialized
   by a cross-process OS file lock (`$TMPDIR/shine-admin.lockfile`). The persistent file is never
   deleted to reclaim ownership; a 30-second wait timeout returns an error.
5. **Manifest** — `install_core/manifest.rs` upserts an `AppEntry` into `~/.shine/app-manifest.toml`,
   recording dest, content hash, strategy, and **`requires_admin`** (must persist — uninstall
   routes on it; see lessons entry 2026-07-04). Successful saves from mutation commands write
   `schema_version = 1`.
6. **Result and report** — the App adapter records safe file/receipt canonical targets, logical
   resources, statuses, effects, and diagnostic codes in `shine-core`'s `LifecycleResultV1`.
   App upgrade, hooks, implicit teardown, embedded preset-cache, and purge join the same result;
   hook and teardown failures retain their non-fatal command semantics. CLI-private presentation
   events flow through a writer-backed reporter, and stale cleanup confirmation uses the frontend
   interaction adapter. Reusable results never include absolute destinations, content, raw errors
   or child output, environment values, or secret values.

## App uninstall

Top-level explicit `app/` and `shell/` uninstall targets dispatch directly to their lifecycle
handlers. Bare names resolve through Core inventory, including receipt-only targets; ambiguity
requires an explicit namespace. Source availability never substitutes for uninstall ownership.

Reverse of install, driven entirely by the manifest — never by re-scanning presets:

1. Look up the `AppEntry` by dest in `~/.shine/app-manifest.toml`.
2. Remove the installed static Copy (sudo path if `requires_admin = true`) or remove only the
   receipt-declared top-level keys for JSON merge.
3. Restore `<name>.shine.bak` if one exists.
4. Remove the manifest entry.

CLI App and Shell uninstall share a completion policy over typed lifecycle outcomes. Preserved,
conflicting, or failed outcomes produce an incomplete summary and a nonzero actual-command exit,
even when independent resources were removed. Successful preview remains zero. App local file
reports carry the receipt strategy solely for removal/backup/managed-key guidance; stable lifecycle
results and snapshot-bound approval remain unchanged. Shell conflict reporting distinguishes
retained files from receipt cleanup and offers no force override.

## Declarative App action and recovery

`planner/app_actions.rs` remains the approved factory entry. It validates approval before reading
Host state, resolves each static file from the captured snapshot and initial manifest, then evaluates
direct JSON relocation, Copy relocation, JSON merge or Copy branches in that order. Pure JSON
subset operations come from `runtime/managed_json.rs`, shared by App hashing adapters, planners and
executors/recovery. Its extraction changes no bytes, hashes, parsing/error order or missing/partial
subset semantics. Host reads/writes, exact approval, journal publication and receipt transitions
remain in the existing domain phases (ADR 0125).
Desired bytes remain local in memory alongside request/Plan/manifest/file references; branches retain
their source/destination/backup/rollback observation order, skip rules and diagnostics. One common wrapper retains Action/operation IDs and file ordering. Ordinary Create/
Update step matching remains target/resource/action based; relocation additionally requires its typed
step kind. Uninstall and appended upgrade stale-prune still use `app_remove_actions.rs`, including
its existing manifest reloads. The factory requires only `FileSystemObservationHost`; construction
locals grant no execution authority. See
[ADR 0121](../decisions/0121-resource-specific-app-action-factories.md).

Approved App install routes two deliberately narrow creation cases through the Roadmap Phase 4
executor: a static Copy whose manifest receipt is absent and whose destination is either absent or
an unowned regular file with an absent fixed backup path. Approved install and upgrade also route
an unchanged receipt-owned, in-place static Copy replacement
through the executor. Ordinary uninstall also routes an unchanged, receipt-owned, unprivileged
static Copy through the executor, including restoration of an unchanged canonical persistent
backup. Forced uninstall of a user-modified file uses a separate action for the same static Copy
boundary. Administrator static Copy create, update, and removal reuse these actions with privileged
path mutations under the administrator lock. JSON merge install, in-place update, ordinary remove,
and forced remove use key-owned actions that preserve unrelated current values. App upgrade stale
pruning reuses the same removal actions when the receipt-owned state is unchanged; a missing
destination removes only its receipt, while user-modified stale state remains preserved. Static
Copy relocation uses one action for the old receipt/path/backup, new absent destination, and new
receipt. JSON relocation uses a separate action for the old receipt/object/rollback, absent new
destination, separate old/new managed-key sets, and replacement receipt. Generators retain their
existing executor and explicit opaque classification:

```text
approved PlanV1
  → include App journal write/remove infrastructure permissions
  → regenerate and validate the exact Plan after approval
  → ActionIrV1(CreateManagedFile; destination + desired hash, no bytes)
    or CreateManagedFileWithBackup; destination + backup + original/desired hashes, no bytes
    or UpdateManagedFile; destination + rollback + previous receipt/mode + before/after hashes,
       no bytes
    or RelocateManagedFile; old destination + optional backup + rollback + absent new destination +
       old/new receipt identities and hashes, no bytes
    or RelocateManagedJson; old destination + rollback + absent new destination + separate
       old/new key/subset/receipt identities, no JSON values
    or RemoveManagedFile; destination + rollback + previous receipt/mode/hash, no bytes
    or RemoveManagedFileWithBackup; destination + persistent backup + rollback + both
       modes/hashes + previous receipt fields, no bytes
    or ForceRemoveManagedFile; destination + optional persistent backup + rollback + distinct
       receipt/current hashes + current/backup modes, no bytes
    or MergeManagedJson; destination + rollback + managed keys + whole-file before identity +
       previous/desired managed-subset hashes, no JSON values
    or RemoveManagedJson; destination + rollback + managed keys + whole-file before identity +
       distinct receipt/current managed-subset hashes, no JSON values
  → every static Copy action binds requires_admin; derive Administrator permission when true
  → validate action permissions are included in the approval
  → acquire host cross-process operation lock
  → for requires_admin static Copy, keep the administrator lock across every check/mutation/commit
    and use privileged write/move/remove/mode operations for destination, persistent backup, and
    rollback paths
  → refuse an existing app-operation-journal.toml
  → atomically persist prepared journal with original PlanApprovalV1
  → atomically create the absent destination, rename original to persistent backup then create,
    rename previous managed bytes to `.shine.rollback` then replace and restore their mode,
    or rename an ordinary/forced uninstall destination to `.shine.rollback`, then optionally move
       `.shine.bak` to destination; JSON actions rename an existing whole object to rollback but
       write only a managed-key merge/removal result
  → atomically persist applied action state
  → App lifecycle atomically persists the matching manifest receipt state
    (owned receipt for create/update; safe receipt absence for remove)
  → removal durably records `receipt-committed` in the journal after manifest save
  → commit re-reads and matches that receipt state
  → update/removal commit removes unchanged transaction rollback material, then removes the journal
```

If execution is interrupted, recovery is separate from ordinary lifecycle planning:

```text
read versioned journal + App manifest + destination + optional persistent/transaction backup
  → matching durable creation receipt => preserve manifest-owned destination/backup; clean journal
  → matching durable update receipt => preserve destination; remove only unchanged rollback material
  → otherwise build app-recovery Plan bound to exact journal, manifest, and current path bytes
  → absent-create: remove only unchanged desired bytes
  → backup-create: restore only original/missing, missing/original, or desired/original safe state
  → managed-update: use the same safe states with `.shine.rollback` and the previous receipt
  → managed-remove: exact old receipt restores unchanged rollback; receipt-committed plus safe
    receipt absence removes it; bare receipt absence restores the old receipt and file
  → backup-restoring managed-remove: before receipt commit, restore only exact
    managed/original/missing, missing/original/managed, or original/missing/managed path states;
    after commit keep the exact user destination and remove only unchanged managed rollback
  → forced managed-remove: before receipt commit, restore the exact user-modified rollback to the
    destination and reverse an optional backup restoration; after commit keep the completed
    uninstall state and remove only exact user-modified rollback material
  → JSON merge/remove: exact rollback supplies prior managed-key values, but recovery changes only
    those declared keys in current JSON and preserves unrelated values; absent-path creation removes
    the file only when no unrelated keys exist
  → committed JSON removal: preserve the now user-owned current object, even if a formerly managed
    key was reintroduced, and remove only exact rollback material
  → privileged removal recovery: request Administrator only when the selected safe state mutates a
    protected path; authorize after recovery Plan approval, then use privileged move/remove
  → privileged create/update recovery follows the same rule for protected destination,
    persistent-backup, and rollback mutations
  → any changed destination, backup, rollback material, or receipt => blocked/preserved
  → `shine app recover` renders explicit destination + operation-journal steps
  → default-No approval, or `--yes` for non-interactive recovery
  → acquire operation lock
  → regenerate and revalidate the same recovery Plan
  → remove or restore only unchanged transaction-created/rollback bytes
  → remove journal
```

The journal contains Action IR identities, paths, hashes, modes, state and the original approval,
never managed/original content or secret plaintext. See
[ADR 0048](../decisions/0048-separate-action-ir-and-explicit-recovery.md),
[ADR 0050](../decisions/0050-backup-aware-app-creation-recovery.md),
[ADR 0051](../decisions/0051-transactional-app-managed-file-update.md),
[ADR 0052](../decisions/0052-transactional-app-managed-file-remove.md),
[ADR 0053](../decisions/0053-transactional-app-backup-restoring-remove.md),
[ADR 0054](../decisions/0054-transactional-forced-app-managed-file-remove.md),
[ADR 0055](../decisions/0055-privileged-app-removal-reuses-typed-transaction.md), and
[ADR 0056](../decisions/0056-privileged-app-create-update-reuse-typed-transactions.md), and
[ADR 0057](../decisions/0057-key-owned-json-merge-transactions.md),
[ADR 0063](../decisions/0063-transactional-app-json-relocation.md), plus
[ADR 0062](../decisions/0062-transactional-app-static-copy-relocation.md), plus
`docs/declarative-action-recovery-prd.md` before extending it to opaque actions.

Ordinary App install/upgrade/uninstall/refresh/artifact mutation never recovers implicitly. When
their planner observes the journal, the blocked Plan directs the user to `shine app recover`.
Read-only status/update inspection does not remove the journal. Missing or invalid journals,
unsupported schemas, opaque actions, and post-interruption user changes fail without mutation; the
CLI reports only a safe rollback count and journal-cleanup outcome.

## App update (`shine update`)

App update uses `FrontendService` inspection over the captured active categories, App manifest and
effective env. The CLI derives `AppRow` values from the same `AppFileInspection` records that Core
projects into `LifecycleOutcomeV1`. Ordinary inspection never runs generators; explicit evaluation
assesses each file once without a second run solely to build the structured result. No independent
CLI test-only assessment remains (ADR 0127). Manifest-owned current files are `unchanged`; source,
new-file, relocation, or missing-destination work is `pending`; user-modified destinations are `conflict` with a safe code
and preservation effect. Preview effects describe resource/receipt work without copying the
assessment's absolute paths or content into the reusable result.

## App upgrade (`shine upgrade`)

Core App upgrade re-applies presets (including re-running transforms
with the *current* `[env]` values) through `app/upgrade.rs`. Its private completion enum distinguishes
unjournaled work, journaled updates and journaled relocations. Only the last owns old-source cleanup;
other relocated paths retain the existing uninstall/rollback behavior. Journaled variants retain the
execution capability through immediate manifest save and transaction commit, then release it at the
same per-file boundary. Receipt-save failure leaves the journal for explicit recovery (ADR 0128).
The operation visits every manifest-tracked install and cleans up stale entries
whose preset no longer exists. `shine upgrade app/<category>` selects manifest entries before the
stale/update loop, so no other app category can be mutated. Shell and managed-sys targeted upgrades
apply the same pre-mutation filtering at their own category/item boundaries. `env/upgrade.rs` does
the same for env-templated content. This is why changing an env var requires `shine upgrade` to
take effect in installed files.

Manifest identity for app files is the preset `source`, while ownership checks remain destination-
based. If metadata changes a source's effective destination, upgrade journals one receipt
replacement spanning both paths. Static Copy binds the old destination/backup/rollback and absent
new destination; interruption before the new receipt restores exact old state. JSON merge instead
binds separate old/new managed-key sets and restores/removes only those keys, preserving unrelated
current values at both objects. A modified owned resource or occupied new destination blocks
relocation without creating a duplicate manifest entry.

Managed sys resources participate in the same flow. `shine update` compares the desired built-in
resource receipt derived from the active env against `sys-manifest.toml`; `shine upgrade` then
re-applies only recorded, profile-enabled managed resources and replaces the receipt after
convergence. Aggregate upgrade does not implicitly compose the Sys shell profile; use explicit
`sys profile enable/disable` for that state. For split DNS,
the receipt comparison includes the normalized domain, DNS servers, and platform resource path.
Update and sys-info output render those receipt differences field by field (`old -> new`) so the
user can inspect the pending system change before granting administrator access to upgrade.
The managed-file driver compares only its desired destination and content hash with the recorded
receipt and emits safe field labels rather than paths or content.

Managed apply/upgrade/uninstall loads the independently versioned `sys-manifest.toml` before
resource, elevation, or composed-profile mutation. Read-only update uses the same typed receipt
comparison to produce both the existing field-difference rows and `pending` `sys/<item>` outcomes.
Built-in drivers return typed resource/backup effects and typed user-modification conflicts; the
adapter never classifies reusable results by parsing `detail` or raw errors. No-op upgrade does not
rewrite the receipt merely to refresh metadata. Bootstrap execution, profile enable/disable, and
composed-profile sync remain outside the structured managed-resource result. Explicit profile
mutation has its own `sys-profile-enable` or `sys-profile-disable` security Plan: it binds live
detection for enable, the run manifest, desired enabled set, generated profile files, and shell
configuration state before writing either the profile or receipt.

Managed Sys presentation also flows through the CLI reporter. Item ownership is rendered before
the interaction adapter requests administrator authorization, preserving prompt context without
making terminal or privilege APIs part of the reusable lifecycle result. The renderer retains the
managed action and dry-run mode: preview headings include `(dry-run)`, planned changes use
`→ would update/remove`, and executed removal uses `✓ removed`. Core status enums, receipt
serialization, and lifecycle results retain their existing contracts.

`shine update --diff` expands stale shell/app rows, while `shine update <TARGET>` resolves one
installed shell/app through the same aliases as `shine info` and prints only its stale files. Each
row carries structured pending changes: content, source/destination relocation, a new file,
deployment metadata, or command-entry refresh. Only content changes invoke `info`'s effective-
content renderer; structural changes are rendered field by field. Inline diffs require valid UTF-8
without NUL bytes and are capped at 256 KiB per side. Embedded versus external preset selection,
transforms, and manual-generator behavior stay identical to `shine info --diff` and the upgrade
operation. Target mode returns after the config check and does not perform the binary release check;
managed sys resources keep their structured receipt differences instead.

## Generated app files

An app `[[files]]` entry may declare
`generator = { script, runtime, env, when_env, auto }`. The static `source` remains a
safe fallback and stable manifest identity. When `when_env` exists in the active `[env]` table, an
approved install, upgrade, or explicit refresh may run the generator and use its UTF-8 stdout as
the effective source before normal transforms and install strategies. Ordinary read-oriented
inspection never runs it; explicit evaluation may execute it. `app/assessment.rs` owns named execution
policies, while `app/inspect.rs` separates filesystem observations from pure status/change
projection. Default inspection requires only `FileSystemObservationHost`; explicit evaluation
retains the process/trust gates and performs the same one-pass assessment before reading installed
content. Install preview uses the transformed static fallback without executing code (ADR 0124).
Inspection regressions use this production path through `FrontendService`, including copy/JSON
ownership, missing/new/relocated files, manual-generator snapshots and lifecycle projection. The CLI
keeps presentation tests and its real-host refresh integration; removed test-only status helpers
cannot serve as the behavior baseline (ADR 0127).
Explicit refresh delegates to `app/refresh.rs`; `app/refresh_selection.rs` shares generated-file
selection and exact receipt admission with the planner. Each caller loads its own current manifest
at the original observation boundary; no review-time selection crosses into approved execution.
Preparation selects exact receipt-owned generated files and completes administrator authorization; per-file apply retains generator/transform/read/
install/upsert order and continues after a file failure. Completion saves the shared manifest once
when any file changed, then runs post-upgrade hooks. The approved caller retains the lifecycle lock
and fresh approval gate throughout. A final receipt-save error still prevents hooks and leaves the
already-applied file writes with the previous durable receipt; no new transaction or automatic
rollback is introduced (ADR 0130).

The execution and reporting rules remain:

1. `shine app install` always materializes first, then reuses the normal
   manifest hash and atomic file installer.
2. `auto` defaults to true; automatic generators may materialize during an approved
   `shine upgrade`. Ordinary status/update reports `app_generator_not_evaluated` when dynamic
   output cannot be known without execution. `info`/`update --run-generators` executes selected
   generators once, applies transforms in memory, and computes status/diffs without writing.
3. `auto = false` makes implicit status local-only and excludes the file from upgrade, but explicit
   `--run-generators` evaluation includes it.
   When that explicit evaluation finds different desired content, the shared inspection result
   retains `app_manual_refresh_required`; update/status render the exact source-scoped `app refresh`
   action and do not fold the file into upgrade targets. If other files in the category are
   upgradeable, both actions remain visible.
   `shine app refresh <category> [source]` explicitly refreshes only
   manifest-owned generated files, with an optional `--force` for user changes. Refresh reviews an
   `app-refresh` Plan that binds manifest ownership, live destination state, generator inputs, and
   potential post-upgrade hooks. Embedded generator Plans also bind the runtime-script
   materialization path before generator execution. The CLI passes that exact reviewed request,
   including opaque secret input versions, through preparation and Core's final pre-mutation Plan
   regeneration.
4. An existing managed destination is the last-known-good snapshot when a
   generator fails; a first-time enabled generator failure is fatal.
5. Only `generator.env` values are injected. External preset or overlay generator code requires a
   matching `app/<category>` scoped trust grant. Execution is deadline- and
   output-size-limited. Failed generators may emit one exact allowlisted diagnostic token; Core
   maps recognized tokens and validated HTTP status numbers to fixed safe descriptions and discards
   all other stderr, so URLs, response bodies, credentials, and arbitrary process output cannot
   enter the error report.
6. A Bun generator is resolved against the physical category that supplied its effective script.
   Embedded temporary scripts use `--no-install`; an external/overlay script uses
   `--install=fallback` only with a valid `package.json` + `bun.lock` pair in that category.
7. External evaluation still requires snapshot-scoped trust. Per-file evaluation failures continue
   through the remaining selection and cause a nonzero command result after all statuses render.

The Surge generator downloads the Base64 URI list selected by each file: `SURGE_SUBSCRIPTION_URL`
for `subscription-proxies.conf`, or `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` mapped into the script as
`SURGE_SUBSCRIPTION_URL` for `wget-cloud-proxies.conf`. It converts supported SS/VMess/Trojan nodes
and writes bare policy declarations. Both files declare `auto = false`
so it runs on install (including `--replace-managed`) or explicit refresh, not ordinary
status/upgrade passes. Its `Subscription` group
loads that file through `policy-path`; other groups reuse the nodes through
`include-other-group=Subscription`. VLESS and unsupported transports are
counted and skipped without logging credentials.

## App artifact build (`shine app artifact apply <app-id>`)

Artifact execution is fully separate from install/upgrade — it never runs automatically; see
[ADR 0009](../decisions/0009-app-artifact-build-explicit-command.md). The CLI reviews a specialized
`app-artifact-apply` Plan, then Core re-plans and validates approval before the following executor
flow for an app preset's `[artifact].script`:

1. Resolves the category from the immutable Preset snapshot and materializes the embedded category
   cache only after approval when embedded mode is active.
2. Resolves the script's location: an overlay's `app/<name>/<script>` wins when that exact script
   exists; otherwise the source (built-in or external) script is used. This lets an overlay keep
   local policy files while inheriting a generic built-in artifact.
3. Creates (idempotently, before spawning) `SHINE_STATE_DIR` (`<shine_dir>/state/app/<name>`)
   and `SHINE_CACHE_DIR` (the OS cache dir via
   the `directories` crate — `<os-cache>/shine/app/<name>`, the same crate/pattern
   `env/workspace/cache.rs::cache_path` already uses for its own per-project cache).
4. Injects only `[artifact].env` mappings whose sources are listed by the category's
   `[permissions].environment`, using the current stored values without decryption, plus fixed
   `SHINE_APP_*` contract variables. The fixed variables win on collisions. Planning fingerprints
   plain values by hash and requires opaque versions for secret-classified names; neither enters
   Plan serialization.
5. Runs the script with `current_dir` set to the resolved app directory and inherited stdio (not
   captured like `post_upgrade` hooks), so build output streams live; a nonzero exit becomes a
   real `Result::Err` instead of being swallowed.
6. For Bun artifacts and teardown, the final script source selects the dependency policy: embedded
   or unlocked external code uses `--no-install`; a locked external/overlay category uses
   `--install=fallback`. Resolution does not alter the cwd, environment contract or permission gate.

For Surge specifically, `shine app install surge` copies the local files and
the generated-subscription fallback into the Surge Profiles dir. The built-in
Bun `build.ts` atomically appends `local-proxies.conf`,
`local-proxy-groups.conf`, and `local-rules.conf` to the corresponding section
includes after `SURGE_PROFILE` is configured. It preserves permissions and line
endings, rejects symlink profiles, and fails when an expected section has no
patchable include. Subscription nodes are not added to `[Proxy]`;
`local-proxy-groups.conf` loads the generated bare policy file through
`policy-path`.

**Teardown (`shine app artifact remove <app-id>`, ADR 0012).** An `[artifact].teardown` script reverses
`build`, sharing the *identical* resolution and env contract above (steps 1–4). Explicit removal
reviews an `app-artifact-remove` Plan and propagates failure. Uninstall includes available teardown
in its lifecycle Plan, runs it best-effort before the file-removal loop, and safely skips blocked
external teardown without stopping owned-file removal. Surge ships a symmetric built-in Bun
`unbuild.ts`; other app
presets may still keep artifact-specific reversal logic in an overlay.

**Lifecycle hooks (`CoreRuntime::run_app_hooks`).** `post_install` (fired by `install`, including
`--replace-managed`) and `post_upgrade` (fired by upgrade or explicit generator refresh) run once
per changed category, require target-scoped trust for external code, and remain non-fatal. A command
hook runs direct argv with its declared env allowlist. A script hook resolves native/Bun code from
the immutable snapshot, is planned and materialized inside the parent lifecycle operation, and
receives its declared env plus the fixed `SHINE_APP_*` contract. It never creates or inherits a
separate artifact approval.

## Shell install / uninstall

The Shell source/deployment model, canonical target parser, external mode, and versioned manifest
are Core-owned. The CLI deployment module consumes those types while retaining the current
distribution adapter for embedded `rust-embed` assets and terminal presentation.

Embedded install transactionally patches selected category assets into the managed cache and
links executables into `~/.shine/bin/`. Core profile reconciliation places PATH setup, source-command
wrappers and supported completion registration in a managed profile; the shell config contains a
sentinel-guarded source entry. Uninstall removes only Shine-managed symlinks/files and deletes the
sentinel block precisely. CLI `shells/profile.rs` retains path and activation-hint adapters only.

Profile mutation tests exercise approved install/uninstall and the production completion entry.
Completion acquires the Shell lifecycle lock, derives source commands from installed receipts and
existing launchers, and calls runtime-scoped profile helpers. The legacy sentinel regression checks
exact update status, preservation of surrounding text and unchanged receipt bytes. These tests no
longer depend on CLI append/remove wrappers (ADR 0139).

For external presets, `external_shell_mode = "snapshot"` first materializes the effective
base/overlay category under `<shine_dir>/installed/shell/`; update compares that snapshot with the
active source and upgrade refreshes it. Explicit `live` mode points raw commands at the external
category. Materialization skips every `node_modules/` directory but preserves `package.json` and
`bun.lock`.

Snapshot deployment accepts snapshot or development trust; live deployment accepts development
trust only. Existing live launchers are preserved after upgrade, while inspection reports targets
that need development-trust review. A category snapshot replacement expands the Plan to all
installed commands that consume the shared tree and checks every affected target grant before the
first mutation. Uninstalled siblings remain available only and do not gain launchers or grants.

Every Bun launcher includes an explicit package policy. Embedded commands and unlocked external
commands use `--no-install`. When the physical category owning an effective external/overlay script
contains both lock files, the launcher uses `--install=fallback`; the Shell manifest records this
mode and a combined content hash. Snapshot changes are applied by upgrade. Live execution reads the
current package files immediately, while status reports that its receipt and launcher need refresh.
A transformed live launcher calls the manifest-constrained internal renderer on each
invocation, then executes or sources the atomically refreshed file under `rendered/`.

## Managed Sys apply and recovery

Approved managed operations, bootstrap, profile changes, and recovery acquire `sys-lifecycle.lock`
before fresh approval validation, retaining it through shared manifest persistence and cleanup.
Profile synchronization uses the same lock. Managed-file roundtrip tests enter through
`plan_managed_sys` and `run_managed_sys_approved`; the former test-only standalone mutation path
has been removed (ADR 0133). The privileged transaction lock is acquired afterward; a manifest
change while waiting invalidates an approval that observes it (ADR 0106).

`sys/lifecycle` selects current preset items or a distinct receipt-only removal, then performs input
and administrator preflight, resource execution, and outcome persistence. Batch items retain their
original order; a later failure leaves earlier committed resources intact. Transaction preparation
uses fresh observations under the approved facade's existing lifecycle lock (ADR 0116).

Managed-file and split-DNS apply/update/uninstall first derive typed actions and an exact previous
and desired `SysRunEntry`. Core writes `sys-operation-journal.toml` before the first resource
mutation, persists the desired receipt, records receipt commit, and then removes exact rollback
material and the journal. Managed files reuse the create/update/relocate/remove state machines;
split DNS binds the complete previous and desired typed state rather than shelling out during
recovery.

A pending journal blocks later mutating Sys Plans. `shine sys recover` captures the journal,
manifest, resource, and rollback observations in a fresh `sys-recovery` Plan, revalidates approval
under the operation lock, and restores only fingerprint-matching previous state before receipt
commit. After commit it keeps the desired resource and cleans exact rollback. Explicit
`sys profile enable/disable` also journals its shell rc changes, but owns only per-phase Shine
sentinel blocks; recovery preserves unrelated current file content.

Profile active/base/new/merge file reconciliation retains the established three-way merge and is
marked non-transactional in its Plan. Bootstrap profile composition, package/provider calls, and
bootstrap scripts are likewise outside managed Sys recovery and retain their explicit opaque or
unsupported-rollback classification.

## Workspace environment export

The workspace facade routes dotenv import/export to `workspace/dotenv.rs`, sealing to
`workspace/sealing.rs`, local execution to `workspace/run.rs`, and immutable broker requests to
`workspace/broker_snapshot.rs`. Model validation, full source compilation and encrypted cache IO
have separate modules. The local hybrid preference/cache path remains local-only; broker capture
and release never load personal preferences. Shared `sources.rs` and `SensitiveValues` continue to
own precedence and plaintext lifetimes (ADR 0115).

`shine env workspace export --format dotenv` resolves one explicit mode through the same ordered
source paths as `env run`, but deliberately excludes inherited process variables and `--with`
injection. The default path parses only `[plain]` values and does not decrypt payloads; a later
secret declaration removes any earlier plain value with the same key. `--include-secrets` switches
to the normal sealed-source compiler, then writes the standalone plaintext result through an atomic
owner-only file on Unix. Export never edits or removes the workspace definition or source files.
Local run, export and broker release share a pure resolver of the winning declaration and source
index. The sealed compiler still validates/decrypts every source, including shadowed declarations.
The ordered-source regression uses `capture_sources` then `compile_captured_sources`, shared with
workspace run, instead of the retired test-only export forwarding method (ADR 0140).
Compiled/explicit maps own their cleanup through `SensitiveValues`; serialized dotenv and decrypted
cache plaintext use `Zeroizing`. Errors and cancelled consumers drop the same guards. Child execution
drops its owned maps before propagating a nonzero process status (ADR 0114).
An early `symlink_metadata` check diagnoses existing output entries. Final publication without
`--force` uses `atomic_write_new` or `atomic_write_private_new`, which stages and syncs content
before Core's `finalize_temp_new` atomically rejects any existing destination. A competing writer
during decryption cannot be overwritten. Force uses the corresponding replacing writer; failed
publication cleans its staging file, and secret bytes are private from the first write on Unix.
Serialization uses literal double, single, or backtick quotes with an absent delimiter, never
JSON-style escaping. Double quotes are used only without literal backslashes or double quotes.
Dollar signs, CR, NUL, and values without a safe delimiter fail before output publication because
Node and Bun dotenv readers cannot preserve them with the same escaping rules. Import likewise
rejects dollar syntax, including bare `$NAME`; unquoted hashes start comments. Import recognizes
single-line backtick delimiters before comment parsing and preserves their literal content;
unterminated/multiline quotes and non-comment trailing content fail before publication.

## Sys bootstrap (`shine sys bootstrap`)

Sys driver/status types, receipts, resource outcomes, `sys-manifest.toml`, split-DNS, bootstrap,
and profile orchestration are Core-owned. Managed mutations return Contract v1; bootstrap and
profile retain their separate typed domain reports.

Core selection resolves explicit ordered items, a named selection profile, or the existing
interactive/default path through the interaction port. Explicit items accept only `mode = "init"`,
deduplicate by first occurrence, and never widen to sibling items.
`sys_bootstrap.rs` retains the batch/item coordinator. Private `selection.rs`, `provider.rs`,
`detection.rs` and `execution.rs` own selection, named fixed provider invocations, presence probes and
bounded installer/result handling. Provider preview and administrator requirements use the same
invocation description. Every selected item and composed profile is preflighted before the first
probe/installer or administrator prompt; execution stops after the first failed item. Successful
items retain their existing additive profile and receipt handling (ADR 0128).

Every executable sys manifest declares `version = 2`. Each init item has both `[items.detect]` and
`[items.install]`; Rust performs the read-only detection, invokes a fixed Homebrew/APT/Winget
provider argv or one per-item script, limits runtime/output, detects again, and produces the
canonical `sys/<item>` outcome. A v1 or unknown manifest version fails before detection, elevation,
installer execution, or profile writes.

Inspection, preview, preflight, and profile composition read the immutable Sys snapshot without
materializing it. Script existence is proven by the logical snapshot entry. After authorization,
script execution creates a private `<shine_dir>/runtime/sys/<os-id>/<invocation>/` copy of the
captured category, restores captured helper executable intent, and runs that copy. It removes only
its own invocation after success or error; other invocations and legacy runtime files remain untouched.
Creation and cleanup permissions share the category runtime scope; random IDs never enter the Plan.
Neither external-source paths nor `Path::is_file` can reopen ambient preset state after capture.
Bounded Unix processes have cancellation guards for their process groups; inherited foreground
terminal input is handed to the child and restored after it finishes or is cancelled (ADR 0100).

Successful bootstrap items set `profile_enabled` in `sys-manifest.toml`.
`core/src/runtime/sys_profile/compose.rs` combines base pre/post content with all enabled item
integrations in stable manifest order. Core reconciles the two generated files before its profile
block module updates the existing pre/post sentinels. Composition
happens once after item execution, and render failure leaves the last installed profile intact.
`sys_profile/files.rs` reads phase inputs and normalizes comparisons; `reconcile.rs` retains the
ordered active/base writes and pending-artifact cleanup. `merge.rs` owns bounded Git invocation;
`three_way.rs` owns the pure fallback algorithm. Non-LF inputs and composed templates bypass Git.
Git exit 0 commits its clean output, exit 1 retains its conflict output for review, and other exits
or a missing Git executable use the fallback. Other spawn errors stop before profile writes.
Conflicts write only `.new` and `.merge`, preserving active/base bytes. A later write failure can
leave an earlier phase/file write applied; this remains ordered reconciliation, not a transaction
(ADR 0124).
Sentinel regressions enter `update_sys_shell_profiles` with explicit configuration and Host; phase
EOL regressions call `install_sys_profile_phase_with_template` with observed templates. Removing
ambient-Host test wrappers does not remove BOM/CRLF, legacy blocks or merge recovery behavior
(ADR 0140).
`sys profile enable/disable` changes only this activation state and generated profile content.

Shine does not run update checks for bootstrap software. Homebrew, APT, Winget, mise, rustup, or
the applicable upstream tool owns package versions and upgrades. Global `shine update` / `shine
upgrade` remain limited to Shine configuration and managed sys resources.

Top-level `shine list` reads current-OS entries with `managed = true` directly from
`sys-manifest.toml` for its installed-only `System Configs` section. It does not call the live
desired-state checker; `shine update` remains responsible for showing only pending managed
changes.

## Maintenance advisories (`shine state check` / interactive invocation)

`cli/src/maintenance/` owns this advisory flow independently of the version gate:

1. Before normal config loading, eligible terminal-stderr invocations derive a scope from the
   selected absolute config root, effective home, native config root and platform. No Presets or
   secrets are loaded. `state check` and the hidden worker route before config loading as well.
2. Under a nonblocking state lock, load the private atomic cache in
   `<shine_dir>/maintenance/<scope-hash>/checks.json`. Missing/corrupt/foreign-build state is a miss.
   Rules choose every invocation/minute/hour/day independently from inline/background execution.
3. Snapshot existing background findings and reserve due heavy rules with a ticket and 120-second
   lease. Release the lock before observing registration files (100 ms limit) or native processes.
4. Commit inline results only if the generation still matches; deduplicate findings by action and
   remind at most daily on stderr. Only background findings already present at invocation start
   are eligible. Spawn the current executable's hidden worker with absolute config scope and ticket.
5. The worker detaches its process session/group and stdio, takes a nonblocking worker lock, validates
   pending tickets, and runs read-only native probes (10-second child timeout, bounded output).
   Each result is committed under the state lock only if generation, rule revision and ticket match.
   The whole worker is bounded to 30 seconds; failures retry no sooner than one minute, and lost
   workers become retryable after the lease expires. Locks are never unlinked for reclamation.
6. Explicit `state check` supersedes in-flight work, runs current rules synchronously, repeats findings
   and reports incomplete probes. Successful `clear serve` advances the generation and resets its
   action state in the selected config scope, so an older worker cannot resurrect a reminder.
   If a newer check or cleanup supersedes an explicit check, it prints a retry warning and exits
   nonzero without a completion summary; it cannot commit its obsolete findings.

No advisory worker applies changes, executes Presets, or decides trust/approval/release gates.

## Update check (`shine update` / background check)

`cli/src/update_check/` (`mod.rs` core + cache, `github.rs` API/auth, `upgrade.rs` install flow):

1. Reads `~/.shine/` cache file; if fresh (24 h TTL, `UPDATE_CACHE_TTL`), no network call.
2. Honors a **rate-limit cooldown**: when GitHub returns a rate-limit error, the
   `rate_limited_until_unix_secs` timestamp (per auth mode) is cached and later checks short-circuit
   until it passes.
3. Otherwise fetches the latest release from GitHub and stores it in the cache.
4. Version-check failures are tolerated in `update_check::maybe_notify` (called from `cli/src/dispatch/mod.rs`) —
   a failed check must never break the primary command the user actually ran.

`shine self upgrade --channel preview` targets the moving `preview` tag instead of the latest
stable `v*` release.

## Git-managed preset pull

`git_pull.rs::handle_pull` resolves the effective `presets_dir` and any *manually linked* overlay
(`presets_overlay_dir`) to their Git roots, de-duplicates shared repositories, and validates every
worktree before running `git pull --ff-only`. Dirty worktrees, detached HEADs, missing upstreams,
and pull failures stop the operation. `update --pull` and `upgrade --pull` pull first, reload
`Config`, then check or apply presets so updated project and environment configuration takes effect
immediately. Successful pulls are summarized as one line per repository (commit range plus short
file stats), while raw Git progress is hidden unless the parent update/upgrade command is verbose.
Failed pulls always include captured Git diagnostics; non-Git and duplicate sources are only shown
verbosely.

A **shine-managed Git overlay** (`presets_overlay_git`) is handled separately, *before* the
fast-forward loop, by `git_pull::sync_managed_overlay` against `<shine_dir>/overlay`. On first use
it clones `--depth 1` via a temp sibling dir + atomic rename (a failed clone never leaves a
half-populated overlay). On subsequent runs it **force-mirrors**: fetch depth 1 from the configured
URL and branch (remote `HEAD` when omitted), update `origin` only after fetch succeeds, then
`git reset --hard FETCH_HEAD` and `git clean -ffdx`, so the checkout equals the remote tip even
across rebases/force-pushes, discarding tracked edits, untracked/ignored files and nested repositories
(the managed overlay is read-only by design). Before fetch and again before reset/clean, verify that
the managed directory and its `.git` entry are ordinary directories and that Git's canonical
worktree/metadata roots match them exactly. Reject parent-repository discovery, linked checkouts
and worktree metadata files. Git subprocesses remove inherited repository selectors while retaining
transport/authentication settings.
The fetch runs before the reset, so an unreachable remote leaves the previous checkout intact and
usable. `shine preset overlay link --git <url>` writes the config and clones immediately;
the command is global-only and rejects an active project before either effect because project
configuration does not load Git source keys and the mirror is shared. The CLI routes Git links
through read-only loading before initialization/advisories, including when global state is absent.
Manual path linking remains
available per project. Inherited global mirrors can still be inspected and pulled within projects.
`configured_targets` deliberately excludes the managed dir from the fast-forward path. See
[ADR 0010](../decisions/0010-git-managed-overlay.md).

## Config discovery

`config/discovery.rs` priority chain (highest first):
`SHINE_CONFIG_DIR` env → `SHINE_PRESETS` env (presets dir only) → `presets_dir` in
`config.toml` → default `~/.shine/`, without project config. An explicit project `presets_dir`
overrides `SHINE_CONFIG_DIR`'s presets default; `SHINE_PRESETS` still wins with project config.
Project-local configs inherit unset keys from the global
config (see lessons entry 2026-07-04 on inheritance). `Config` saves use
`shine_core::sync_table`; directed key edits use `toml_edit`. Both preserve TOML comments. CLI runtime assembly makes selected roots
lexically absolute before snapshot capture and planning; it does not resolve ordinary directory
symlinks or require missing destinations to exist. RealHost metadata rejects special filesystem
entries, so capture cannot open a FIFO, socket, or device as a regular preset payload.

Local Preset/overlay link commands require representable native operands before optional creation,
expand through the captured config home, and reject unrepresentable canonical results before the
existing config save. Full expansion reports an invalid home only when the input actually uses it;
neither operand nor home conversion can select a replacement-character sibling.

Env resolution captures and validates every selected file, then merges builtin defaults → active
project-resolved overlay → global `[env]` → global env file → nearest project `[env]` → project env
file. Strings and detailed entries missing `description` inherit descriptions independently of
values; explicit empty descriptions clear them. Config retains raw layers, effective values, value
and description provenance, and input preimages. Builtins never persist during initialization or
unrelated saves; historical entries remain explicit. Global and sparse project saves apply deltas
against raw active-file definitions, including explicit equal-value pending edits.

Env list/get/explain and directed writes load without runtime initialization or advisory workers.
Runtime assembly copies the already resolved `Config.env` exactly once. Read adapters use
`EnvConfig::from_config` synchronously; they do not reload or initialize env configuration after
capture. Core exposes no general mutable Context to CLI adapters. Sys bootstrap may supply an
explicit proxy map with the consuming `with_proxy_env` assembly method; review and fresh approval
validation bind those inputs as before (ADR 0113).
Write options choose scope and file independently of the current source. Preflight protects
implicit shadowed writes and rejects managed mirrors/retired force, then captures target and all
configuration inputs. Encryption runs outside locks. Commit acquires the canonical target-parent
`.<filename>.shine.lock`, rechecks inputs/context, and privately atomically updates only the selected
key. Config saves, proxy config mutations and recipient migrations coordinate on that same lock.
Deletion requires a target-local definition and reveals inheritance; it never seeks another layer.
Post-save reload reports effective provenance and value change separately from saving. Errors after
saving state that the file is already saved. External editors/Git are not transactionally locked.
See [ADR 0109](../decisions/0109-env-baseline-and-directed-writes.md).

## CLI preset asset reads and export

Config loading resolves overlay roots on its own `Config` instance without registering global
asset state. Env catalog reads and `preset export` construct `PresetAssetReader` with that
instance's `active_presets_overlay_dir()`. The reader merges sorted embedded/overlay paths and
reads matching overlay files first, retaining embedded fallback for absent/unreadable overlay
files. Manual roots retain precedence; managed roots become active only when the checkout exists.
The env catalog's full-external mode still uses `Config::preset_path` and never borrows a missing
catalog from the binary. Export still selects embedded assets plus overlay, even with a full
external source; it does not export that external tree. `preset copy` and Core bootstrap use the
pristine embedded APIs. All extraction retains the same output-root, link, permission and atomic
publication checks. These CLI readers are live views; Core lifecycle execution continues to use
host-captured immutable snapshots. See [ADR 0119](../decisions/0119-explicit-preset-asset-readers.md).

## Dynamic shell completion

`main.rs` calls `completion::complete_from_env` before Clap parsing, Tokio startup, config
initialization, schema warnings, or update checks. Registration and each Tab request build the
Clap command graph and attach dynamic candidates for active preset resources, recorded sys items,
and saved tasks. Candidate callbacks use `config::discover_runtime_paths_read_only`, which mirrors
global/project preset and overlay inheritance synchronously without creating `~/.shine`, then read
only the small preset metadata or runtime manifest needed for the active argument. Parse or I/O
failures are tolerated and never break the user's shell.
Embedded candidate enumeration/metadata reads use pristine APIs, then union candidates from the
explicitly discovered overlay. Loading another `Config` in the same process cannot change those
sources. Sys candidates retain baseline IDs even when an overlay replaces the matching manifest.
Full-external mode also unions its filesystem base with that overlay for category, Shell command,
App capability and Sys ID candidates; no callback constructs a runtime or loads trust state.

## Environment command runner

`env/workspace.rs::handle_run` optionally loads and merges workspace environment sources, then
adds each repeated `--with KEY[=ALIAS]` value from the active config `[env]`. Explicit values use
the same lookup as `env secret export` (`KEY_SECRET` decrypted first, then plaintext `KEY`) and override
both workspace values and inherited process variables. Without a discovered or explicit
workspace, at least one `--with` is required. The merged environment is applied only to the
spawned child process, whose exit status is propagated by Shine.

## Transparent environment proxies

Install/uninstall acquire the config-scope `proxy-lifecycle.lock`, then the target configuration
lock before reading shared launcher/manifest state. Project installs use the same lifecycle lock
as global installs and uninstall. Uninstall captures the complete launcher set, rechecks it before
rule mutation and each removal, and preserves changed or newly appeared paths with an error.

Installation prepares all launcher bytes, captures regular-file preimages (including Unix modes),
and parses the exact captured configuration/manifest before writing. Each write rechecks its
preimage; reported failures roll back touched files in reverse order. Rollback accepts only this
installation's expected bytes and mode states, preserves concurrent edits, and reports incomplete
restoration. Configuration and restoration temporary files start private. This is an in-process
failure rollback, not durable crash recovery ([ADR 0101](../decisions/0101-proxy-install-failure-rollback.md)).
Proxy regression fixtures now use the same `install_proxy_files` owner, rather than separately
writing rules, receipts and launchers. Conflict assertions include configuration and receipt bytes.
Global uninstall keeps project rule overrides intact; privacy tests exercise both scopes through
production install/disable/uninstall, with captured environment restoration (ADR 0140).

`shine env proxy install <command> --with KEY` places a Shine-owned PATH shim
ahead of the real CLI. The shim records the resolved real executable and absolute installation
runtime directory, passes that directory through `--config-dir`, and invokes
`env::proxy::exec`, which reloads the effective global/project configuration,
selects that command's `[[env_proxy]]` rule, and injects only its declared
values (`KEY_SECRET` decrypted first, then `KEY`) into the child process.
Project rules replace the global rule for the same command. The shim never
exports values to the parent shell and never scans all `_SECRET` values.
Each rule defaults to `enabled = true`; `shine env proxy disable <command>`
retains the shim but bypasses environment-value lookup and secret decryption entirely. Missing
applicable rules also pass through without injection, so project-only proxies work outside that
project. Target discovery compares canonical directory identities to exclude aliases of Shine bin,
but retains the executable filename for rustup-style dispatch.

After waiting for the child, proxy execution reuses `workspace::finish_command_status`, also used
by environment commands and live Bun. Numeric exit codes remain unchanged; Unix signal termination
becomes `128 + signal`. All injection branches share this completion path; spawn errors retain their
existing context and child streams remain inherited.

## SSH transfer control channel

The SSH agent listener retains the tracked task set. `agent/connection.rs` performs the unchanged
Hello/version exchange and checks tokens before status, transfer or broker dispatch. Broker
authorization and nonce replay protection remain in `ssh/broker.rs`; tokens alone release no
secret. `agent/transfer.rs` retains path resolution, local/remote tool probing, fallback and
preview order; `transfer/argv.rs` builds operands only from wire paths and connection options only
from trusted session context. `transfer/relay.rs` owns child supervision (ADR 0126).

Transfer control connections monitor peer closure concurrently with child output and final wait.
Failure terminates and reaps the local transfer child, and cancellation drops a child guard. Unix
children have a dedicated process group so cleanup also terminates descendants. Already-copied
files remain the transfer tool's responsibility; Shine does not roll them back.

POSIX-remote SSH session resources have an ownership guard before listener setup. On an SSH
spawn/wait result, the handler stops and awaits the accept loop, drains transfers, and drops the
session directory before returning an error or propagating the exit status. Setup failures also
drop the guard. Literal IPv6 destinations select SCP, whose bracketed operand preserves the full
address and optional user/zone; this avoids the macOS system rsync parser's first-colon split.

## SSH environment forwarding

`ssh/arguments.rs` parses the option/destination/command boundary into a named local result and
builds transport argv without reinterpreting user options. `ssh/command.rs` owns pure POSIX quoting
and UTF-16LE/Base64 PowerShell wrapping. The session facade keeps capture, broker setup, listener
ownership, PID handoff, Ctrl-C handling, shutdown and exit propagation; moving pure construction
changes neither command bytes nor the distinct POSIX/Windows transport behavior (ADR 0130).

`ssh::handle_ssh` resolves each `--with KEY[=ALIAS]` from the exact plaintext key in the active
config `[env]`; it never performs the secret-first fallback used by `env run`. Each
`--with-secret KEY[=ALIAS]` instead loads `KEY_SECRET` and decrypts it through the tag-routed
secret backend. Duplicate aliases and Shine's own SSH/session variable names are rejected. The
default `--remote-shell posix` flow joins the resolved map, `SHINE_SSH_*`, and terminal-theme hint
in the quoted `env ... sh -c` wrapper and creates the transfer listener/`-R` channel. The explicit
`--remote-shell windows` flow instead encodes a PowerShell script as UTF-16LE Base64, probes for
`pwsh.exe` (PowerShell 7), and falls back to `powershell.exe` (Windows PowerShell 5.1); it sets only
the session hint, theme, and selected variables and creates no listener, reverse forward, or
`shine local` channel. Its interactive child loads the selected PowerShell's normal profile so
managed PATH entries and source-command wrappers are available; an explicit remote command keeps
`-NoProfile` for deterministic execution. Values are session-only but
necessarily exposed in process argv/environments on the local and remote hosts; see
[ADR 0014](../decisions/0014-explicit-ssh-env-forwarding.md) and
[ADR 0015](../decisions/0015-windows-ssh-environment-forwarding.md).

## SSH on-demand secret broker

`shine ssh --secret-broker` attaches a local `ssh::broker::BrokerSession` to the existing reverse
control channel. The local process freezes the active config, encrypted direct-secret inputs, and
the policy file for the lifetime of that SSH session; private-key operations remain local.
The private `ssh/broker/interaction.rs` port owns terminal mechanics only. Session mode, exact
allow-list/policy matching, nonce claims and the operation lock stay in `BrokerSession`. A successful
confirmation returns an owning guard that spans subsequent decryption or enrollment persistence;
dropping it restores termios and resumes SSH on success, error or cancellation. The blocking terminal
task also retains its restoration guard if its async caller is cancelled before completion (ADR 0128).

For a direct remote request, `shine env run --no-workspace --secret-broker --secret KEY[=ALIAS]
-- <argv>` sends only the requested mapping and argv. The local agent checks the SSH session's
`--allow-secret` list, pauses the interactive SSH child, restores the local TTY for an explicit
confirmation, decrypts only `KEY_SECRET`, and returns the selected values. Direct requests always
confirm, including in a trusted workspace session.

For a workspace request, `env/workspace.rs` reads the workspace and all selected source files once
into a bounded `WorkspaceSnapshot`. The request carries the exact bytes and SHA-256 identities,
mode, complete declared-secret set, requested release mapping, and argv. `env/broker.rs` accepts
only an exact match in `<shine_dir>/ssh-secret-broker.toml`; the local agent then confirms unless
`--trust-remote-session` was explicitly set, decrypts captured sealed payloads, and returns only
policy-approved effective secrets. The remote merges those values with non-secret entries from the same
snapshot bytes through `merge_broker_values` and injects them only into the child process.
The retired standalone plain-value extraction entry has no role in release or merging (ADR 0137).

Both sides preserve source order across plain and secret declarations: later plain suppresses an
earlier secret response, later secret removes an earlier plain value even if unreleased, and only
the winning approved secret source contributes a returned value. The policy still binds complete
source hashes and declared-secret lists, including shadowed declarations.
The shared declaration resolver never filters that policy input. Broker release validates every
captured source and payload before returning approved winners; the existing public map return is
an explicit transfer of plaintext ownership to the broker caller.

`shine ssh --secret-broker-inspect` displays one remote description without decrypting or writing.
`--secret-broker-enroll --trust-remote-metadata` may create a local policy after local confirmation
when the operator explicitly trusts the remote. The safer normal path is
`shine env broker policy add`, generated from a trusted local workspace checkout. See
[ADR 0024](../decisions/0024-ssh-on-demand-secret-broker.md) and the
[secret-broker PRD](../../ssh-secret-broker-prd.md).

Policy describe/add/update/diff accept either explicit repeated `--release KEY` or
`--release-all-declared`. The latter expands the current snapshot into a sorted explicit release
array; no wildcard reaches disk or the wire. Any new declared secret changes the source identity
and fails closed until policy update. When no trusted local checkout exists,
`--secret-broker-enroll --trust-remote-metadata --update-policy NAME` previews a full diff and may
replace exactly one same-mode/same-argv allow while preserving the named policy's local identity
fields; a concurrent local policy edit aborts the write.
Local policy updates use the same exact-target recheck after human confirmation. Every writer
acquires `ssh-secret-broker.lock` before its final store read and retains it through persistence;
unrelated policy changes are merged from that fresh read, never from the review-time store.

## Personal task runner (`shine task run` / `shine run`)

`task::handle_run` loads `<shine_dir>/tasks.toml` (`task::manifest::TaskManifest`), looks up the
named task's saved argv, appends any `-- EXTRA...` args, and spawns it with
`std::process::Command` — **directly, with no shell** — inheriting the caller's stdio and
environment. When the task has an explicit `cwd`, saved as a canonical absolute path by
`task save --cwd`, the handler validates it and sets `Command::current_dir`; missing `cwd` retains
the caller's current directory for backward compatibility. The child's exit code is propagated verbatim (`std::process::exit(code)`; on Unix a
terminating signal becomes `128 + signal`), never wrapped in an anyhow error, so the task's own
exit semantics survive Shine in the middle. `shine run <NAME>` is a top-level alias routed to the
same handler. `task::handle_save` validates the name (`[A-Za-z0-9._-]`, letter/digit start) and
rejects an empty command or a duplicate without `--force`; `info`/`list` render the argv back to a
copy-paste-safe line by shell-quoting shell-significant arguments. Save and delete acquire
`<shine_dir>/tasks.lock` through `FileSystemHost::acquire_operation_lock` before loading the
manifest, retain it through validation and atomic save, and release it without unlinking.
The shared 30-second timeout fails without changing the manifest; reads remain lock-free.

## Secret backend routing (GPG / age)

Both adapters encode/decode Base64 in process using the existing Rust crate; only the encryption
backend and its identity plugins require external commands. Decoding strips ASCII whitespace,
validates standard padded Base64 completely, then writes ciphertext to the existing private
temporary file. See [ADR 0083](../decisions/0083-in-process-secret-base64.md).

Every call site that decrypts a stored secret (`env secret decrypt`, `env secret export`, workspace
`env secret seal`/`env run`) goes through `secret::decrypt_with_config(ciphertext, config)`, which inspects the
ciphertext for an `age:` or `hybrid:` prefix (`secret::parse_tagged_ciphertext`) and dispatches to
`secret::age`/`secret::gpg` or the authenticated hybrid envelope accordingly; untagged ciphertext is always GPG. Decryption never
reads `Config::secret_backend` — only the tag decides. Encryption (`env secret encrypt`, workspace
`env secret seal`) instead resolves a `secret::EncryptRecipients` (CLI `-r`/`--backend` > workspace
`env.encryption` > `config.toml` `gpg_recipients`/`age_recipients`/`secret_backend` > GPG default)
and calls `secret::encrypt_secret`, which tags age output and leaves GPG output untagged. See
[ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md) for the full rationale.
Shell export resolves `KEY_SECRET` before plaintext `KEY`, decrypts the selected secret through
that route, then rejects NUL before rendering any shell code. Its diagnostic omits the value;
normal values keep the existing per-shell quoting, while get/decrypt retain raw output.
The Unix `shine-env-export` source helper captures output in a subshell, forwards successful code
to eval, and substitutes a literal numeric return on failure without evaluating partial output.
`shine env secret identity init [--touch-id]` generates a local age identity file
(`age-keygen`/`age-plugin-se keygen --recipient-type=tag`), passing native output/staging paths
without text conversion. Custom-output hints capture the absolute path before keygen runs and
print a TOML assignment only for UTF-8 paths; generation does not apply the hint to config.
Every age path verifies age 1.3 or
newer. `state migrate` rewrites configured `age1se` recipients to equivalent native `age1tag`
recipients while preserving their decoded public key and leaving ciphertext untouched. The
`--phone` form on Windows and experimental macOS instead invokes the standalone plugin's
transactional setup with explicit `--recipient-type tag` by default (`phone` is opt-in),
preflighting age 1.3 before tag pairing. It validates the requested recipient type, Bech32
encoding and tag public-key structure, and consumes only the versioned public identity-path/recipient result before
atomically appending the stub path to global `age_identities`. `Config::resolved_age_identities()`
merges the legacy `age_identity` path with that ordered list and passes each path separately to
`age -i`; an explicit project identity setting replaces the global set. Shine never discovers or
manages phone-plugin TPM/Secure Enclave keys, replay, locator, pairing, recovery, or cleanup state. See
[ADR 0075](../decisions/0075-phone-identity-setup-handoff.md).


All workspace sealing rejects symbolic-link sources (and multiple hard links on Unix) before
capturing any sources and rechecks file type/link count before atomic replacement. Ordinary GPG
encryption and decryption force `--output -`; empty successful encryption output is rejected
before serializing a sealed source, preserving pending plaintext on failure.
The plain-only regression enters `handle_seal` with explicit workspace/file paths, and controlled
backend tests enter `decrypt_with_config`; no separate test sealing orchestration or config-free
decryption wrapper remains (ADR 0137).

Hybrid workspace sealing captures policy and source bytes under cooperating workspace/source
locks, resolves exact GPG public encryption keys with option files disabled, and preflights both
encryption branches before decrypting old payloads. A fresh OS-random data key encrypts one payload;
GPG and age wrap its versioned nonce-bound key record. The complete header and wrappers are AEAD
associated data. Private temporary output is prepared before final workspace/source comparisons
and replacing rename. Partial multi-file success is reported, never rolled back over external edits.

Local `hybrid_decrypt_backend` selects one hybrid unwrap after any broker release approval.
Local workspace run/seal/export may override the global default from `shine.config.local.toml`
beside the selected workspace; broker snapshots do not load or transmit this personal file.
Without a preference, capability-only preflight selects a sole candidate or requires a local TTY
choice. Failure terminates without backend fallback. Runtime captures mode inputs once; hybrid
policy or consumed tags select the local encrypted cache path. It validates captured hybrid wire
structure before selecting/decrypting a cache, freezes the backend, and binds the snapshot, backend
and workspace recipients inside the encrypted cache. Cache writes use only the selected tool;
cache decryption cancellation never retries from sources. See [ADR 0085](../decisions/0085-local-hybrid-preferences-and-cache.md)
for local trust and cache boundaries, and [ADR 0084](../decisions/0084-hybrid-secret-envelope.md)
for the unchanged envelope and sealing concurrency contract.

## One-operation external code consent

Trusted human-facing lifecycle review may obtain one-time consent for exact external Preset code
alongside Plan confirmation. CLI confirmation mentions external-code consent only when a typed
external/overlay boundary is listed; internal-only Plans use the ordinary apply question. Temporary grants are private Core runtime state, cleared after review,
and carried only in the non-serializable ApprovedOperation. Execution reinstalls those exact
identities and re-plans against fresh input; changed source, state or configuration rejects approval.
No persistent grant is written. Automatic `--yes` and read-only/AI review use the ordinary path and
remain blocked without persistent trust. Shell live always needs Development trust. Explicit
info/update generator evaluation retains its persistent-grant requirement. Author statements are
review data, not trust identity; env/admin remain per-operation executor contracts. See ADR 0091.
