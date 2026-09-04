# Data Flows

End-to-end flows that span multiple modules and are not visible in any single file. For module
ownership and the per-command routing table, see [`module-map.md`](module-map.md) — this file only
records the cross-module sequences and their gotchas.

## Pure security Plan assessment

`shine-core::plan` defines the Phase 3 approval contract used by protected mutation.
`runtime::planner` consumes one immutable `PresetSnapshot`, a validated App/Shell/managed Sys,
exact Sys bootstrap, App refresh/artifact, or Sys profile request, captured runtime inputs,
manifests, receipts, and live resource observations. It emits
ordered semantic steps plus a required permission set; missing declarations, uncomputable
permissions, or a blocked step make the Plan non-ready. Planning cannot invoke host mutation or
Preset code, and its output carries no content, env values, secret plaintext, raw errors, or raw
command arguments.

The effective Preset snapshot hashes sorted logical paths, bytes, and trust layers without hashing
its physical checkout root. Target selection from immutable request/Preset input occurs before
host-state reads. Filesystem and split-DNS observation traits expose manifests, receipts, live
resources, launchers, and system state without exposing write/process/privileged/apply methods.
Planners hash every outcome-affecting observation using the same framed SHA-256 builder. Plain env
values contribute hashes; secrets contribute only caller-supplied opaque handles or versions.

```text
validated target + immutable Preset snapshot
    → observation-only manifest/receipt/live-state capture
    → ownership and lifecycle assessment
    → merge typed effects + explicit target permissions
    → ordered payload-free PlanV1 + state/Preset digests
```

Generator, hook, artifact, bootstrap, and profile-code triggers are modeled as conservative
`execute` plus potential resource steps; the code is never run during planning and missing or stale
target-scoped trust remains a blocker. A supported receipt can drive uninstall after source
disappearance, but cannot recreate missing teardown code.
CLI review creates approval for one exact ready Plan. Apply deliberately follows:

```text
capture current source/state → regenerate Plan → match approved fingerprint + permissions
    → execute existing Core lifecycle → return LifecycleResultV1
```

App, Shell, managed Sys install/upgrade/uninstall, exact Sys bootstrap, App refresh/artifact, and
Sys profile enable/disable route through this flow. Untargeted `shine upgrade` renders the three
final lifecycle Plans together, confirms once, and prevalidates all three before protected mutation
starts. `upgrade --pull` pulls and reloads first. Existing dry-run/status remain separate
preview/inspection paths. Scoped external-code trust, ownership, and administrator authorization
remain additional gates.

Sys bootstrap uses the dedicated `sys-bootstrap` Plan operation rather than a lifecycle install.
Interactive or profile selection resolves to an exact ordered item list before planning. The pure
planner observes command/path presence without executing detection, binds run-manifest,
environment/proxy and profile state, and derives package-provider, script, administrator and
profile permissions. Approved execution re-plans before any detection command, installer,
materialization, profile write, or receipt mutation. Its existing domain report remains separate
from `LifecycleResultV1`.

Permission declaration schema v1 is parsed from the same immutable snapshot: one App category
table, one table per Shell command/platform variant, and one table per Sys item. Static validation
checks version, placement, structured paths, payload-free identities, and duplicates without
executing Preset code. Typed metadata continues to describe Core-bounded effects; explicit tables
record additional capabilities. Pure planners combine both sources into the required/declared
resolution used by `PlanV1`; missing or uncomputable capabilities make that Plan non-ready and
protected execution fails closed.

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
snapshot and returns a content-free report plus process-local candidate bytes.

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

App and Shell use a first-install lifecycle request. Sys partitions the validated manifest into
managed and init items, then emits separate managed-install and bootstrap sections. The synthetic
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

## Preset pack

Packing validates one immutable category snapshot, then separately walks the physical category with
an observation-only host so ignored trees and symlinks cannot evade policy. After policy checks,
Core sorts logical files, excludes the author-only fixture, builds the versioned hash/mode manifest,
and encodes fixed-metadata tar.gz bytes. The CLI performs only the requested atomic output write.

```text
category → immutable validation → physical policy scan
    → sorted logical files - shine.test.toml → shine.bundle.json
    → fixed tar/gzip metadata → bytes + SHA-256 → explicit CLI output
```

## Scoped external-code trust

`core::runtime::trust` derives requirements only from the immutable logical Preset snapshot. Each
requirement binds a canonical App/Sys target, capability kind, digest of the relevant code inputs
and effective trust layers, and the exact target permission set. `core::trust::evaluate_trust`
matches that requirement against versioned grants without consulting project configuration.

```text
immutable code inputs + trust layers + declared permissions
    → TrustRequirementV1
    → exact match against global owner-only trust.toml
    → trusted or stable missing/stale decision
    → decision bound into Plan state → separate one-shot Plan approval
```

The CLI loads `~/.shine/trust.toml` before constructing `RuntimeContext`. `shine trust grant`
derives and renders the current requirement, confirms with default No, then atomically stores only
the reviewed identities. Code, layer, or permission changes do not match. Legacy coarse booleans
are diagnostic-only and never create grants.

## Shell install and uninstall

Shell lifecycle targets are either a category (`utils`) or one command in a category
(`utils/shine-env-export`). Embedded sources and external snapshots remain category-scoped shared
deployment material so a command can consume sibling resources, while launchers and
`shell-manifest.toml` receipts are command-scoped.

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
mutations save schema v1, read-only status/update does not rewrite it, and a future version fails
before mutation. The Shell adapter emits one `shell/<category>/<command>` outcome per installed or
selected command. Read-only update maps typed row changes to `pending` plus write-preview effects;
foreign launcher ownership is `conflict`, not pending, and upgrade preserves that launcher and its
receipt. Shared cache/snapshot/rendered effects attach to affected commands without turning source
presence into installation evidence.

Shell execution emits CLI-private presentation events instead of writing terminal output. The
terminal renderer owns shared upgrade-section state, while writer-backed recording tests pin
quiet/verbose sections, conflicts, profile hints, and stdout/stderr routing.

## App install (`shine app install <category>`)

`CoreRuntime` owns the complete App lifecycle from an immutable preset snapshot: metadata and
destination resolution, one-pass assessment, generators, transforms, Copy/JSON merge, relocation,
ownership, hooks, artifacts, embedded cache, manifest persistence, and Contract v1 mapping. The
CLI supplies resolved config plus `rust-embed` bytes. Shared runtime bootstrap discovers any
external/overlay tree through `FileSystemHost`, constructs the immutable snapshot, and submits the
request; the CLI renders typed events/reports. There is no frontend-specific directory walker,
prepared-file path, or CLI fallback executor.

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
   by a cross-process advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as mutex).
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

Reverse of install, driven entirely by the manifest — never by re-scanning presets:

1. Look up the `AppEntry` by dest in `~/.shine/app-manifest.toml`.
2. Remove the installed static Copy (sudo path if `requires_admin = true`) or remove only the
   receipt-declared top-level keys for JSON merge.
3. Restore `<name>.shine.bak` if one exists.
4. Remove the manifest entry.

## Declarative App action and recovery

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

App update loads active categories, the App manifest, and effective env once, then derives each
`AppRow` and `LifecycleOutcomeV1` from the same `AppFileAssessment`. This keeps terminal filtering
compatible while preventing an automatic generator from running a second time solely to build the
structured result. Manifest-owned current files are `unchanged`; source, new-file, relocation, or
missing-destination work is `pending`; user-modified destinations are `conflict` with a safe code
and preservation effect. Preview effects describe resource/receipt work without copying the
assessment's absolute paths or content into the reusable result.

## App upgrade (`shine upgrade`)

Core App upgrade re-applies presets (including re-running transforms
with the *current* `[env]` values) to every manifest-tracked install, and cleans up stale entries
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
making terminal or privilege APIs part of the reusable lifecycle result.

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
inspection never runs it; explicit evaluation may:

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
   output-size-limited.
6. A Bun generator is resolved against the physical category that supplied its effective script.
   Embedded temporary scripts use `--no-install`; an external/overlay script uses
   `--install=fallback` only with a valid `package.json` + `bun.lock` pair in that category.
7. External evaluation still requires snapshot-scoped trust. Per-file evaluation failures continue
   through the remaining selection and cause a nonzero command result after all statuses render.

The Surge generator downloads the Base64 URI list in
`SURGE_SUBSCRIPTION_URL`, converts supported SS/VMess nodes, and writes bare
policy declarations to `subscription-proxies.conf`. It declares `auto = false`
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
3. Creates (idempotently, before spawning) `SHINE_APP_HTTP_DIR` (`<shine_dir>/http/app/<name>`),
   `SHINE_STATE_DIR` (`<shine_dir>/state/app/<name>`), and `SHINE_CACHE_DIR` (the OS cache dir via
   the `directories` crate — `<os-cache>/shine/app/<name>`, the same crate/pattern
   `env/workspace.rs::cache_path` already uses for its own per-project cache).
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

Embedded install transactionally patches selected category assets into the managed cache, links
executables into `~/.shine/bin/` (`bin_links.rs`), and appends a sentinel-guarded PATH block to the
shell config (`shells/profile.rs`). Uninstall removes only Shine-managed symlinks/files and deletes
the sentinel block precisely.

For external presets, `external_shell_mode = "snapshot"` first materializes the effective
base/overlay category under `<shine_dir>/installed/shell/`; update compares that snapshot with the
active source and upgrade refreshes it. Explicit `live` mode points raw commands at the external
category. Materialization skips every `node_modules/` directory but preserves `package.json` and
`bun.lock`.

Every Bun launcher includes an explicit package policy. Embedded commands and unlocked external
commands use `--no-install`. When the physical category owning an effective external/overlay script
contains both lock files, the launcher uses `--install=fallback`; the Shell manifest records this
mode and a combined content hash. Snapshot changes are applied by upgrade. Live execution reads the
current package files immediately, while status reports that its receipt and launcher need refresh.
A transformed live launcher calls the manifest-constrained internal renderer on each
invocation, then executes or sources the atomically refreshed file under `rendered/`.

## Managed Sys apply and recovery

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

`shine env workspace export --format dotenv` resolves one explicit mode through the same ordered
source paths as `env run`, but deliberately excludes inherited process variables and `--with`
injection. The default path parses only `[plain]` values and does not decrypt payloads; a later
secret declaration removes any earlier plain value with the same key. `--include-secrets` switches
to the normal sealed-source compiler, then writes the standalone plaintext result through an atomic
owner-only file on Unix. Export never edits or removes the workspace definition or source files.

## Sys bootstrap (`shine sys bootstrap`)

Sys driver/status types, receipts, resource outcomes, `sys-manifest.toml`, split-DNS, bootstrap,
and profile orchestration are Core-owned. Managed mutations return Contract v1; bootstrap and
profile retain their separate typed domain reports.

Core selection resolves explicit ordered items, a named selection profile, or the existing
interactive/default path through the interaction port. Explicit items accept only `mode = "init"`,
deduplicate by first occurrence, and never widen to sibling items.

Every executable sys manifest declares `version = 2`. Each init item has both `[items.detect]` and
`[items.install]`; Rust performs the read-only detection, invokes a fixed Homebrew/APT/Winget
provider argv or one per-item script, limits runtime/output, detects again, and produces the
canonical `sys/<item>` outcome. A v1 or unknown manifest version fails before detection, elevation,
installer execution, or profile writes.

Inspection, preview, preflight, and profile composition read the immutable Sys snapshot without
materializing it. Script existence is proven by the logical snapshot entry. After authorization,
script execution atomically replaces `<shine_dir>/runtime/sys/<os-id>/` with the captured category
and runs that staged copy; neither external-source paths nor `Path::is_file` can reopen ambient
preset state after shared bootstrap capture.

Successful bootstrap items set `profile_enabled` in `sys-manifest.toml`.
`core/src/runtime/sys_profile/compose.rs` combines base pre/post content with all enabled item
integrations in stable manifest order. Core reconciles the two generated files before its profile
block module updates the existing pre/post sentinels. Composition
happens once after item execution, and render failure leaves the last installed profile intact.
`sys profile enable/disable` changes only this activation state and generated profile content.

Shine does not run update checks for bootstrap software. Homebrew, APT, Winget, mise, rustup, or
the applicable upstream tool owns package versions and upgrades. Global `shine update` / `shine
upgrade` remain limited to Shine configuration and managed sys resources.

Top-level `shine list` reads current-OS entries with `managed = true` directly from
`sys-manifest.toml` for its installed-only `System Configs` section. It does not call the live
desired-state checker; `shine update` remains responsible for showing only pending managed
changes.

## Update check (`shine update` / background check)

`cli/src/update_check/` (`mod.rs` core + cache, `github.rs` API/auth, `upgrade.rs` install flow):

1. Reads `~/.shine/` cache file; if fresh (24 h TTL, `UPDATE_CACHE_TTL`), no network call.
2. Honors a **rate-limit cooldown**: when GitHub returns a rate-limit error, the
   `rate_limited_until_unix_secs` timestamp (per auth mode) is cached and later checks short-circuit
   until it passes.
3. Otherwise fetches the latest release from GitHub and stores it in the cache.
4. Version-check failures are tolerated in `update_check::maybe_notify` (called from `main.rs`) —
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
half-populated overlay). On subsequent runs it **force-mirrors**: `git fetch --depth 1 origin
<branch>` then `git reset --hard FETCH_HEAD`, so the checkout always equals the remote tip even
across rebases/force-pushes, discarding local edits (the managed overlay is read-only by design).
The fetch runs before the reset, so an unreachable remote leaves the previous checkout intact and
usable. `shine preset overlay link --git <url>` writes the config and clones immediately;
`configured_targets` deliberately excludes the managed dir from the fast-forward path. See
[ADR 0010](../decisions/0010-git-managed-overlay.md).

## Config discovery

`config/discovery.rs` priority chain (highest first):
`SHINE_CONFIG_DIR` env → `SHINE_PRESETS` env (presets dir only) → `presets_dir` in
`config.toml` → default `~/.shine/`. Project-local configs inherit unset keys from the global
config (see lessons entry 2026-07-04 on inheritance). `Config` saves go through
`shine_core::sync_table`, which preserves TOML comments.

## Dynamic shell completion

`main.rs` calls `completion::complete_from_env` before Clap parsing, Tokio startup, config
initialization, schema warnings, or update checks. Registration and each Tab request build the
Clap command graph and attach dynamic candidates for active preset resources, recorded sys items,
and saved tasks. Candidate callbacks use `config::discover_runtime_paths_read_only`, which mirrors
global/project preset and overlay inheritance synchronously without creating `~/.shine`, then read
only the small preset metadata or runtime manifest needed for the active argument. Parse or I/O
failures are tolerated and never break the user's shell.

## Environment command runner

`env/workspace.rs::handle_run` optionally loads and merges workspace environment sources, then
adds each repeated `--with KEY[=ALIAS]` value from the active config `[env]`. Explicit values use
the same lookup as `env secret export` (`KEY_SECRET` decrypted first, then plaintext `KEY`) and override
both workspace values and inherited process variables. Without a discovered or explicit
workspace, at least one `--with` is required. The merged environment is applied only to the
spawned child process, whose exit status is propagated by Shine.

## Transparent environment proxies

`shine env proxy install <command> --with KEY` places a Shine-owned PATH shim
ahead of the real CLI. The shim records the resolved real executable and invokes
`env::proxy::exec`, which reloads the effective global/project configuration,
selects that command's `[[env_proxy]]` rule, and injects only its declared
values (`KEY_SECRET` decrypted first, then `KEY`) into the child process.
Project rules replace the global rule for the same command. The shim never
exports values to the parent shell and never scans all `_SECRET` values.
Each rule defaults to `enabled = true`; `shine env proxy disable <command>`
retains the shim but bypasses config lookup and secret decryption entirely.

## SSH environment forwarding

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

For a direct remote request, `shine env run --no-workspace --secret-broker --secret KEY[=ALIAS]
-- <argv>` sends only the requested mapping and argv. The local agent checks the SSH session's
`--allow-secret` list, pauses the interactive SSH child, restores the local TTY for an explicit
confirmation, decrypts only `KEY_SECRET`, and returns the selected values. Direct requests always
confirm, including in a trusted workspace session.

For a workspace request, `env/workspace.rs` reads the workspace and all selected source files once
into a bounded `WorkspaceSnapshot`. The request carries the exact bytes and SHA-256 identities,
mode, complete declared-secret set, requested release mapping, and argv. `env/broker.rs` accepts
only an exact match in `<shine_dir>/ssh-secret-broker.toml`; the local agent then confirms unless
`--trust-remote-session` was explicitly set, decrypts only the policy-approved release subset, and
sends values back. The remote merges those values with non-secret entries parsed from the same
snapshot bytes and injects them only into the child process.

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
copy-paste-safe line by shell-quoting shell-significant arguments.

## Secret backend routing (GPG / age)

Every call site that decrypts a stored secret (`env secret decrypt`, `env secret export`, workspace
`env secret seal`/`env run`) goes through `secret::decrypt_secret(ciphertext, age_identities)`, which inspects the
ciphertext for an `age:` prefix (`secret::parse_tagged_ciphertext`) and dispatches to
`secret::age`/`secret::gpg` accordingly; untagged ciphertext is always GPG. Decryption never
reads `Config::secret_backend` — only the tag decides. Encryption (`env secret encrypt`, workspace
`env secret seal`) instead resolves a `secret::EncryptRecipients` (CLI `-r`/`--backend` > workspace
`env.encryption` > `config.toml` `gpg_recipients`/`age_recipients`/`secret_backend` > GPG default)
and calls `secret::encrypt_secret`, which tags age output and leaves GPG output untagged. See
[ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md) for the full rationale.
`shine env secret identity init [--touch-id]` generates a local age identity file
(`age-keygen`/`age-plugin-se keygen`). The `--phone` form instead invokes the standalone plugin's
transactional setup and consumes only its versioned public identity-path/recipient result before
atomically appending the stub path to global `age_identities`. `Config::resolved_age_identities()`
merges the legacy `age_identity` path with that ordered list and passes each path separately to
`age -i`; an explicit project identity setting replaces the global set. Shine never discovers or
manages phone-plugin TPM, replay, locator, pairing, recovery, or cleanup state. See
[ADR 0075](../decisions/0075-phone-identity-setup-handoff.md).
