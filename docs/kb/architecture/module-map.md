# Module Map

Use this map to find the owner of a command or behavior. It describes module responsibility, not
behavioral contracts; read [`invariants.md`](invariants.md) and [`data-flows.md`](data-flows.md)
before changing cross-module behavior.

Update this file when modules move, split, merge, or take on a different responsibility.

## Workspace layout

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Workspace manifest and publishable `shine-cli` package root |
| `cli/` | Main `shine` binary plus its library crate |
| `cli/build.rs` | Embedded preset rebuild triggers, build metadata, and Windows MSVC executable stack reserve |
| `core/` | Reusable `shine-core` package with no CLI/Tauri dependency |
| `core/src/lifecycle.rs` | Versioned frontend-neutral lifecycle result envelope and safe effect/status vocabulary |
| `core/src/plan.rs` | Versioned snapshot-bound security Plan, permission resolution, fingerprint, and approval contracts |
| `core/src/action.rs` | Versioned executable Action IR, constructors, opaque provenance and rollback DTOs |
| `core/src/action/contracts.rs`, `action/contracts/` | Exhaustive resource-family routing and domain validation/permission rules; incompatible input fails closed |
| `core/src/frontend.rs` | Versioned redacted Frontend Service contracts and inventory; `frontend/inspection.rs` owns observation-only ordinary inspection/shared report projection and separate trusted explicit evaluation; `frontend/review.rs` dispatches pure Plan review |
| `core/src/frontend/{operations,events}.rs` | Safe journal-state/recovery projection and explicit progress-event redaction; domain executors retain journal parsing and recovery decisions |
| `core/src/frontend/{bootstrap,read_only,trusted}.rs` | Shared source capture, restricted safe report facade, human review and one-shot approval, exact validation and lifecycle/recovery dispatch; `conformance.rs` checks adapter equivalence |
| `core/src/permission.rs` | Versioned target-local Preset permission declarations, normalization, and payload-free identity validation |
| `core/src/trust.rs` | Versioned target-local external-code trust grants, exact matching, and stale-scope decisions |
| `core/src/runtime/` | Internal Core runtime facade, immutable preset inputs, host ports, in-memory host, domain models, manifests, and migrated executors |
| `core/src/runtime/bootstrap.rs` | CLI/UI-shared, host-backed external/overlay discovery and immutable snapshot construction |
| `core/src/runtime/host.rs` | Observation-only filesystem/split-DNS ports plus inheriting filesystem, scoped operation-lock, process, privileged, and system mutation ports |
| `core/src/runtime/launcher.rs` | Borrowed launcher inputs, shared resource preparation, exact comparison, Host installation and ownership probes |
| `core/src/runtime/launcher/templates.rs` | Pure Unix/PowerShell/CMD launcher byte generation, quoting and legacy/current format reconstruction |
| `core/src/runtime/launcher/tests.rs` | Launcher consumer regressions against production Host APIs and prepared transaction resources; no parallel test-only filesystem implementation |
| `core/src/runtime/operation_lock.rs` | Persistent OS file locks and configuration-scoped App/Shell/Sys lifecycle serialization |
| `core/src/runtime/tests/` | Test-only runtime harness and regressions spanning domain modules; `lifecycle_concurrency.rs` covers receipt preservation and approval revalidation under contention |
| `core/src/runtime/tests/refactoring_contracts.rs` | Cross-module App static/generator receipt preservation and Shell read-only interruption/recovery/retry contracts |
| `core/src/runtime/process_scope.rs` | Isolated-process cancellation guard and scoped Unix foreground-terminal handoff |
| `core/src/runtime/review_path.rs` | Shared root-aware path identity encoding for planners and App/Shell/Sys action executors |
| `core/src/runtime/planner.rs` | Pure Plan request types and shared observation/permission accumulators; domain methods live in `planner/` |
| `core/src/runtime/planner/{app_plan,app_convergence,app_file_plan,app_stale_plan,app_specialized,app_code}.rs` | App lifecycle orchestration, per-file convergence, stale receipt assessment, refresh/artifacts and triggered code |
| `core/src/runtime/planner/app_file_plan/{generated,static_file}.rs` | Generated/static permission and step projection after the shared ownership preflight; captured facts remain local to one planning call |
| `core/src/runtime/planner/{shell_plan,shell_removal,shell_convergence,shell_profile}.rs` | Shell orchestration and separate removal, convergence and profile assessment phases |
| `core/src/runtime/planner/shell_convergence/{cache,snapshot,command}.rs` | Ordered category-cache, shared-snapshot/sibling-code and per-command convergence assessment; `command/rendered.rs` assesses rendered output before launchers |
| `core/src/runtime/planner/shell_removal/` | Observation-only legacy/receipt launcher, rendered-file, purge/cache and snapshot removal phases in the original order |
| `core/src/runtime/planner/tests.rs`, `planner/tests/` | Shared in-memory fixtures and domain/contract-grouped planning, transaction, recovery and frontend regressions |
| `core/src/runtime/planner/{sys_managed_plan,sys_specialized,sys_support}.rs` | Managed Sys, bootstrap and profile planning with shared Sys observation helpers |
| `core/src/runtime/planner/approved.rs` | Lifecycle locks, fresh-plan approval revalidation and approved domain entry points |
| `core/src/runtime/planner/app_actions.rs` | Approved App factory routing, per-file inputs, exact step matching and ordered Action IR wrapping |
| `core/src/runtime/planner/app_actions/{copy,json_merge,file_relocation,json_relocation}.rs` | Observation-only resource admission/rechecks and payload-free static Action construction |
| `core/src/runtime/planner/app_remove_actions.rs` | Existing receipt-driven uninstall and stale-prune Action construction |
| `core/src/runtime/planner/tests/app_action_factory.rs` | Mixed resource order/identity/permission, Preserve/generator exclusion, typed relocation and late-state factory regressions |
| `core/src/runtime/planner/{observations,inputs,permissions,code_boundaries}.rs` | State/input capture, permissions, typed code-boundary assembly and Plan finalization |
| `core/src/runtime/app_change.rs` | Shared static App relocation eligibility used by Plan and Action IR construction |
| `core/src/runtime/action_executor.rs` | App journal/result/recovery models; `action_executor/` owns typed apply, commit, receipt and recovery phases |
| `core/src/runtime/action_executor/{authority,file_apply,relocation_apply,json_apply,removal_apply,commit}.rs` | Shared exact approval/IR/permission gate and resource-specific transactions with receipt-gated commit |
| `core/src/runtime/action_executor/commit/{action,files,json}.rs` | Borrowed App-only commit view, exhaustive dispatch and resource-specific rollback cleanup |
| `core/src/runtime/action_executor/{inspection,recovery_plan,recovery_apply}.rs` | Journal inspection, receipt-conflict recovery orchestration and approved reverse-order rollback dispatch |
| `core/src/runtime/action_executor/recovery_{plan,apply}_{file,json,relocation,removal}.rs` | Action-specific read-only recovery review and mutation handlers; rollback semantics remain domain-owned |
| `core/src/runtime/action_executor/{receipts,recovery_assessment,file_ops,journal_io}.rs` | Ownership predicates, exact live-state assessments, privileged path dispatch and durable journal IO |
| `core/src/runtime/shell_action_executor.rs` | Shell transaction contracts and shared resource observations |
| `core/src/runtime/shell_action_executor/receipt_changes.rs` | Named local previous/desired receipt handoffs and their existing payload-free V1 projection; persistence/validation stay at transaction owners |
| `core/src/runtime/shell_action_executor/action.rs` | Exhaustive borrowed Shell-only Action conversion and receipt-marker eligibility shared by journal validation, commit and recovery dispatch |
| `core/src/runtime/shell_action_executor/{apply,commit,journal_io,receipts}.rs` | Paired Action/payload preparation and application, receipt-gated commit and durable journal IO; `apply/prepare.rs` revalidates each resource before journal publication |
| `core/src/runtime/shell_action_executor/commit/tests.rs` | Mixed-resource commit preflight/cleanup, retry, marker eligibility and foreign-domain journal boundary regressions |
| `core/src/runtime/shell_action_executor/{recovery_plan,recovery_apply}.rs` | Recovery orchestration; `recovery_plan/` owns observation-only resource assessments, and `recovery_apply/resources.rs` owns reverse-dispatched rollback handlers |
| `core/src/runtime/sys_action_executor.rs` | Phase 4 managed Sys file, split-DNS, and explicit profile-sentinel actions; Sys receipt transition journal, exact rollback cleanup, and explicit recovery Plan/apply |
| `core/src/runtime/sys_action_executor/{action,resources,file_ops,split_dns,profile,observations,journal_io}.rs` | Borrowed Sys-only action view, exhaustive resource dispatch, domain rollback rules, captured recovery observations and durable journal IO |
| `core/src/runtime/trust.rs` | Derivation of App/Shell/Sys external-code requirements from immutable logical code inputs, resolved defaults, and declared permissions |
| `cli/src/trust.rs`, `cli/src/commands/trust.rs` | Owner-only trust-store persistence plus target-local and current-Preset batch `shine trust` workflows |
| `core/src/runtime/app_script.rs` | Per-invocation captured App script/helper materialization, fixed execution paths, and cleanup |
| `core/src/runtime/app.rs` | App models and typed content/generator states, install/refresh/uninstall, shared code-trust gate, embedded cache and manifest orchestration |
| `core/src/runtime/app/{generator,hooks,artifact}.rs` | Separate generator, post-lifecycle hook and artifact execution contracts; all script entries use the captured-category `app_script` seam |
| `core/src/runtime/app/execution_tests.rs` | Entry-point trust/no-effect gates, allowlisted process requests, generator limits/redaction, hook continuation and artifact I/O/failure regressions |
| `core/src/runtime/managed_json.rs` | Host-free top-level managed-key extraction, merge/remove/restore, comparison and stable subset hash/serialization shared by App assessment, planning, execution and recovery |
| `core/src/runtime/app/assessment.rs` | Snapshot-only content assessment and named install-preview/install/upgrade/evaluated-inspection generator policies |
| `core/src/runtime/app/inspect.rs` | Observation-only ordinary inspection, explicit generator evaluation adapter and pure inspection projection |
| `core/src/runtime/app/inspection_contract_tests.rs` | Production FrontendService inspection/status-to-lifecycle regressions; fixtures seed receipts and bytes without duplicating assessment |
| `core/src/runtime/app/assessment_tests.rs` | Consumer regressions for generator assessment, inspection priority, approved lifecycle reports and last-known-good receipts |
| `core/src/runtime/app/outcome.rs` | Internal typed App file outcomes projected into the existing local report fields |
| `core/src/runtime/app/lifecycle/{install,remove}.rs` | App preparation, per-file apply/receipt commit and finalization; caller retains lifecycle lock and exact approval |
| `core/src/runtime/app/refresh.rs` | Named selected-file/prepared/applied states for explicit refresh; per-file continuation and final receipt-before-hook completion under the approved caller |
| `core/src/runtime/app/upgrade.rs` | Upgrade phases under the lifecycle lock; typed unjournaled/update/relocation completion retains transaction guards through receipt save and commit |
| `core/src/runtime/shell_launch.rs` | Installed-receipt Live Bun preparation under the operation lock; typed launch description |
| `core/src/runtime/app_metadata.rs` | App snapshot parsing and effective destinations, including legacy `shine-dest` compatibility and shared parent-component rejection |
| `core/src/runtime/shell.rs` | Shell models, lifecycle entry points, source-scoped Bun dependency policy, live render, launcher, cache, profile, and manifest helpers |
| `core/src/runtime/shell/metadata.rs` | Snapshot-only metadata parsing, explicit declarations, native discovery and platform/source admission; no Host required |
| `core/src/runtime/shell/inspect.rs` | Observation-only Shell inspection orchestration, per-command evidence, pure changes/status projection and receipt-only records |
| `core/src/runtime/tests/shell_inspection.rs` | Read-only consumer regressions for Shell status/change ordering, missing presets, trust/conflicts and observation failures |
| `core/src/runtime/shell/{cache,rendered}.rs` | Ordered embedded-cache/source selection and rendered-resource preparation with shared-destination consistency; existing effect timing stays with lifecycle orchestration |
| `core/src/runtime/shell/delivery.rs` | Pure captured-input source layout and desired delivery shared by inspection, planning, launcher preparation and receipt construction |
| `core/src/runtime/shell/lifecycle.rs`, `shell/lifecycle/{deploy,remove,report}.rs` | Typed Shell deployment/removal phases and pure lifecycle reporting; preparation retains legacy effects |
| `core/src/action/shell_receipt.rs` | Shared internal receipt delivery validation for manifest reconstruction, Action IR and recovery; persisted DTOs remain unchanged |
| `core/src/runtime/sys.rs` | Managed Sys models, receipt assessment, lifecycle-owned managed-file/split-DNS primitives and run-manifest persistence; roundtrip tests use the approved planner facade |
| `core/src/runtime/sys/inspect.rs` | Observation-only managed receipt inspection and pure desired-versus-receipt details; installed-only inventory filtering belongs to FrontendService, with no detection probes |
| `core/src/runtime/sys/{lifecycle,transaction}.rs`, `sys/lifecycle/` | Current-item/receipt-only selection, preflight, resource execution, result persistence and fresh transaction preparation; approved facade retains the lifecycle lock |
| `core/src/runtime/sys_bootstrap.rs` | Sys v2 batch/item orchestration, preflight, post-detection, profile reconciliation and persistence |
| `core/src/runtime/sys_bootstrap/selection.rs` | Ordered explicit/profile/interactive/default selection without installation effects |
| `core/src/runtime/sys_bootstrap/provider.rs` | Named fixed provider invocation, platform admission, preview and administrator requirement |
| `core/src/runtime/sys_bootstrap/{detection,execution}.rs` | Captured-context presence probes and bounded installer execution/result classification respectively |
| `core/src/runtime/sys_profile/` | Sys profile facade, composition and phase-sentinel/BOM handling in `compose.rs` and `blocks.rs` |
| `core/src/runtime/sys_profile/{files,reconcile,merge,three_way}.rs` | Profile observations and EOL normalization, ordered writes, bounded Git adapter and pure fallback merge respectively |
| `core/src/runtime/validation.rs` | Stable V1 diagnostics/report facade and ordered all-platform validation orchestration |
| `core/src/runtime/validation/{source,permissions,destinations,sys}.rs` | Observation-only scope capture; target-local permission diagnostics; portable destination paths; snapshot-only Sys references and metadata |
| `core/src/runtime/metadata_diagnostic.rs` | Typed metadata error origins mapped to stable validation codes; message prose is not a classifier |
| `core/src/runtime/preset_migration.rs` | Immutable legacy compatibility assessment, safe metadata candidate generation, released-1.x fingerprints, and versioned content-free migration reports |
| `core/src/runtime/inspection.rs` | Typed App/Shell inspection status and structural change vocabulary |
| `core/src/install/` | Core-owned transforms, EOL handling, host-required App manifest persistence, and host-neutral managed-file operations |
| `core/src/persist.rs` | Core-owned synced atomic replacement and no-clobber publication, mode preservation, Windows write-through moves, and versioned TOML helpers |
| `presets/` | Embedded shell, app, and OS bootstrap assets |
| `skills/shine-preset-author/` | Portable AI workflow and kind-specific preset author references |
| `docs/manual/` | Default English public manual |
| `website/i18n/zh-Hans/` | Simplified Chinese public manual and UI locale |
| `docs/kb/` | Internal agent knowledge: invariants, flows, decisions, operations, lessons |

## CLI entry and routing

| Path | Responsibility |
|---|---|
| `cli/src/main.rs` | Process entry, Clap parsing, early completion and Tokio setup |
| `cli/src/dispatch/` | Library-owned dispatch; `before_config.rs` owns initialization-free routing, domain modules own App/Shell/Sys/Env/Preset dispatch, and `tests.rs` owns parser/routing regressions |
| `cli/src/lib.rs` | Library module tree; exposes handlers that require lib-level tests |
| `cli/src/commands/cli.rs` | Root Clap parser and top-level argument types |
| `cli/src/commands/*.rs` | Domain subcommand enums and argument types |
| `cli/src/shim.rs` | Canonical `install`/`uninstall <TARGET>` routing and bare-category ambiguity checks |
| `cli/src/init.rs` | `shine init` project-local `shine.config.toml` creation |
| `cli/src/completion.rs` | Static and dynamic shell completion before ordinary CLI initialization |
| `cli/src/list.rs` | Stable installed-list/update/status entry points and the status Shell-attention output boundary |
| `cli/src/list/{query,projection,render}.rs` | Ordered observations and named local query results, pure row grouping/pending/action projection, and terminal/diff presentation |
| `cli/src/info/` | Canonical target resolution, installed data collection, and diff rendering |
| `cli/src/path_display.rs` | Home-relative terminal paths and display-only logical filesystem permission labels |
| `cli/src/colors.rs` | Terminal color helpers |
| `cli/src/output.rs` | Shared command output mode and rendering support |
| `cli/src/presentation.rs` | CLI-private lifecycle events, writer-backed terminal renderer, and interaction ports |
| `cli/src/core_runtime.rs` | CLI settings and embedded-byte supply into the shared host-backed runtime bootstrap and Frontend Service construction; tests cover live overlay readers versus captured snapshots and review authority |
| `cli/src/lifecycle_plan.rs` | Default-No/non-TTY approval policy, input identities and batch prevalidation; retains the public review adapter |
| `cli/src/lifecycle_plan/render.rs`, `render/` | Pure Plan display facade with compact/detailed/bootstrap layouts, conservative permission summaries, code-boundary warnings and semantic labels |
| `cli/src/lifecycle_plan/prepared.rs` | Private reviewed-request handoffs, named three-domain upgrade prevalidation and consuming typed report execution; Core retains fresh approval checks |
| `cli/src/lifecycle_plan/prepared/tests.rs` | Batch shape/domain rejection before capture, all-domain prevalidation before execution and apply-time revalidation regressions |
| `cli/src/lifecycle_plan/classification.rs` | Terminal-only typed grouping of unchanged Plan steps, routine notices and conservative unknown-diagnostic fallback |
| `cli/src/platform.rs` | Platform classification shared across command domains |
| `cli/src/privilege.rs` | Cross-platform administrator/elevation orchestration |
| `cli/src/proc.rs` | Small domain-neutral subprocess helpers |
| `cli/src/persist.rs` | Compatibility re-export of Core persistence helpers |
| `cli/src/shell_quote.rs` | Copy-paste-safe shell argument rendering |
| `cli/src/version.rs` | Version string formatting |

### Command routing table

| Top-level command | Handler |
|---|---|
| `shell list/info/install/uninstall/recover` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `install/uninstall <TARGET>` | `cli/src/shim.rs` → `apps/` or `shells/` |
| `list [--available [KIND]]` | `cli/src/list.rs` or scoped list handlers |
| `info <TARGET>` | `cli/src/info/`; explicit system items delegate to `sys/` |
| `app artifact apply/remove <app-id>` | `cli/src/apps/build.rs` |
| `app refresh <app-id> [file]` | `cli/src/apps/refresh.rs` |
| `app recover` | `cli/src/apps/recovery.rs` → Core explicit recovery Plan/apply |
| `shell recover` | `cli/src/shells/recovery.rs` → Core explicit Shell transaction recovery Plan/apply |
| `sys recover` | `cli/src/sys/recovery.rs` → Core explicit managed Sys recovery Plan/apply |
| `sys list/bootstrap/profile/...` | `cli/src/sys/` |
| `theme sync` | `cli/src/theme/` |
| `clear serve` | `cli/src/legacy_serve.rs`; config-independent retired user-service cleanup |
| `state check`, hidden `__maintenance-worker` | `cli/src/maintenance/`; best-effort advisory scheduling, scoped cache, detached read-only worker |
| `env ...` | `cli/src/env/` plus `cli/src/secret/` |
| `preset export/copy/link/unlink/overlay` | `cli/src/preset_commands.rs` |
| `preset new/validate/lint/plan/test/pack` | Kind template handlers plus the `cli/src/preset_{validation,lint,authoring,test,pack}.rs` adapters |
| `preset migrate [PATH]` | `cli/src/preset_migration.rs` orchestrates `preset_migration/{discovery,assessment,render,apply}.rs`: read-only inputs, same-scope validation, report/diffs, source rechecks/private backup/atomic edits |
| `preset pull`, `update --pull`, `upgrade --pull` | `cli/src/git_pull.rs` plus top-level routing |
| `init` | `cli/src/init.rs` |
| `self install/upgrade` | `cli/src/self_install.rs` facade → `self_install/{binary,release}.rs`, `cli/src/update_check/` |
| `update [TARGET]`, `upgrade [TARGET]` | `self_install/{update,config_upgrade}.rs`; untargeted upgrade retains Shell/App/managed Sys batch review |
| `state migrate` | `cli/src/state.rs` |
| `ssh ...` | `cli/src/ssh/mod.rs` |
| `local download/upload/status` | `cli/src/ssh/remote_client.rs` |
| `task save/run/list/info/delete`, `run <NAME>` | `cli/src/task/` |
| `completions` | `cli/src/main.rs` with `clap_complete` |

## App installation

| Path | Responsibility |
|---|---|
| `cli/src/apps/mod.rs` | App scaffolding, handler re-exports and Core-backed destination tests |
| `cli/src/apps/install.rs` | Core App-install request adapter and terminal rendering |
| `cli/src/apps/uninstall.rs` | Core App-uninstall request adapter and terminal rendering |
| `cli/src/apps/upgrade.rs` | Core App-upgrade request adapter and terminal rendering |
| `cli/src/apps/info.rs` | App list/info status |
| `cli/src/apps/report.rs` | Install/uninstall outcome formatting |
| `cli/src/apps/metadata.rs` | Compatibility re-exports of Core App metadata types |
| `cli/src/apps/refresh.rs` | Core explicit-generator-refresh adapter |
| `cli/src/apps/recovery.rs` | Core explicit App operation recovery Plan/apply adapter |
| `cli/src/apps/build.rs` | Core artifact apply/remove adapter |
| `cli/src/install_core/file_ops.rs` | Test-only compatibility coverage for Core host-backed copy, backup, and restore primitives |
| `cli/src/install_core/manifest.rs` | Compatibility re-export of Core-owned `app-manifest.toml` types |
| `core/src/install/` | App manifest, file ownership primitives, `jsonc-to-json`/`template`, and EOL helpers |

`install_core` retains CLI manifest aliases and test-only file-operation compatibility coverage;
execution primitives and transforms live in `core/src/install/`.

## Shell deployment

| Path | Responsibility |
|---|---|
| `cli/src/shells/mod.rs` | Shell types, shared accessors, handler re-exports |
| `cli/src/shells/deployment.rs` | Hidden live-render and single-process Live Bun launch adapters |
| `cli/src/shells/install.rs` | Core category/command install and upgrade adapter |
| `cli/src/shells/recovery.rs` | Core explicit Shell transaction recovery Plan/apply adapter |
| `cli/src/shells/uninstall.rs` | Category/command uninstall results with sibling/cache and foreign-launcher protection |
| `cli/src/shells/links.rs` | Conflict reporting and category-specific repair guidance for preset/rendered sources |
| `cli/src/shells/report.rs` | Shell list/install/uninstall/upgrade reporting |
| `cli/src/shells/metadata.rs` | Compatibility re-exports of Core Shell metadata types |
| `cli/src/sentinel.rs` | Shared sentinel-block primitives used by shell and sys profiles |

## System configuration

| Path | Responsibility |
|---|---|
| `cli/src/sys/commands.rs` | Sys list/info/status rendering, exact bootstrap selection, and approved Core bootstrap Plan adapter |
| `cli/src/sys/detect.rs` | OS and Linux distribution detection |
| `cli/src/sys/managed.rs` | Managed-resource apply/update/remove/upgrade and structured result adapters |
| `cli/src/sys/model.rs` | Compatibility re-exports of Core Sys models |
| `cli/src/sys/manifest.rs` | Core parser compatibility adapter |
| `cli/src/sys/run_manifest.rs` | Compatibility re-export of Core-owned Sys manifest and receipt state |
| `cli/src/sys/selection.rs` | Selected-item terminal formatting only |
| `cli/src/sys/execution.rs` | Bootstrap reporting and proxy environment |
| `cli/src/sys/render.rs` | System command presentation helpers |
| `cli/src/sys/profile_commands.rs` | Core profile enable/disable adapter and rendering |
| `cli/src/sys/recovery.rs` | Core explicit managed Sys operation recovery Plan/apply adapter |

## Configuration, presets, and runtime state

| Path | Responsibility |
|---|---|
| `cli/src/config/mod.rs` | `Config` model and accessors |
| `cli/src/config/discovery.rs` | Global/project and environment path priority |
| `cli/src/config/load.rs` | Load/init and schema/version layering |
| `cli/src/config/save.rs` | Atomic, comment-preserving, raw-layer config saves |
| `cli/src/config/env_layer.rs` | Env value parsing, removed-file diagnosis and override persistence |
| `cli/src/config/env_resolution.rs` | Raw six-layer env resolution, input snapshots and independent value/description provenance |
| `cli/src/config/env_write.rs` | Explicit write targets, preflight, snapshot checks and shared per-target config locks |
| `cli/src/presets.rs` | Pristine embedded assets plus explicit `PresetAssetReader` overlay enumeration/reads/extraction; shared safe output publication |
| `cli/src/preset_commands.rs` | Preset copy/export/link/unlink/overlay commands |
| `cli/src/preset_meta.rs` | Test-only Core capability-report renderer for public manual parity |
| `cli/src/preset_validation.rs` | Core validation report text/JSON rendering and exit mapping |
| `cli/src/preset_lint.rs` | Core author-quality lint report rendering and strict-CI exit mapping |
| `cli/src/preset_authoring.rs` | Hypothetical authoring-plan text/JSON rendering and platform mapping |
| `cli/src/preset_test.rs` | Declarative fixture report rendering and failed-case exit mapping |
| `cli/src/preset_pack.rs` | Deterministic bundle report rendering and explicit atomic output write |
| `cli/src/preset_schema.rs` | Live Clap help plus Core-generated authoring schema reference rendering |
| `core/src/runtime/authoring.rs` | Same-snapshot static validation and synthetic-host App/Shell/Sys authoring reports |
| `core/src/runtime/lint.rs` | Same-snapshot cross-platform author-quality, portability, and permission-minimization diagnostics |
| `core/src/runtime/fixture.rs` | Versioned `shine.test.toml` parsing and structured authoring-report assertions |
| `core/src/runtime/pack.rs` | Bundle manifest, reproducible tar.gz bytes, and source-tree pack policy |
| `core/src/runtime/schema.rs` | Generated JSON Schemas for authoring reports, fixtures, and bundle manifests |
| `cli/src/git_pull.rs` | FF-only external source pulls and managed overlay mirroring |
| `cli/src/state.rs` | Versioned runtime-state cleanup |
| `cli/src/status.rs` | App inspection-to-terminal row adapter and stable row re-exports; no independent test-only App status engine |
| `cli/src/status/shell.rs` | Pure Shell inspection-to-row projection, stable command selectors/targets and typed CLI status priority |

Config discovery priority is documented as a behavioral contract in
[`data-flows.md`](data-flows.md#config-discovery) and
[ADR 0005](../decisions/0005-config-discovery-priority-chain.md).

## Environment and secrets

| Path | Responsibility |
|---|---|
| `cli/src/env/mod.rs` | `[env]` configuration and substitution core |
| `cli/src/env/commands.rs` | Env CRUD and secret command handlers |
| `cli/src/env/live_bun.rs` | Receipt-prepared Bun execution, declared env injection, and direct-child Unix signal relay |
| `cli/src/env/workspace.rs` | Stable workspace handler facade, discovery and shared path/source resolution |
| `cli/src/env/workspace/{model,dotenv}.rs` | Workspace/source/cache DTOs and validation; dotenv import/export, portable quoting and private/no-clobber publication |
| `cli/src/env/workspace/{sealing,compilation,cache}.rs` | Snapshot-bound sealing, complete source compilation/decryption and encrypted default-backend cache IO |
| `cli/src/env/workspace/{broker_snapshot,run}.rs` | Immutable broker capture/release and local child environment/execution orchestration |
| `cli/src/env/workspace/{tests,hybrid_snapshot_tests}.rs` | Existing workspace/source, dotenv, plaintext-lifetime and hybrid snapshot regressions |
| `cli/src/env/workspace/sources.rs` | Pure ordered plain/secret declaration precedence shared by local compilation, export and broker release |
| `cli/src/env/workspace/values.rs` | Owned environment maps with redacted diagnostics and cleanup on replacement, error and cancellation |
| `cli/src/env/workspace/file_groups.rs` | Version 3 binary selection, seal/verification, export and scoped release; typed operations and preview streams preserve preflight and output contracts |
| `cli/src/env/workspace/file_groups/filesystem.rs` | Root-constrained paths, snapshots, Git exclusions, shared locks, owned-file cleanup and persistence |
| `cli/src/env/workspace/file_groups/platform.rs` | Native private creation, file identity, cancellation, and Unix/Windows command supervision |
| `cli/src/env/catalog.rs` | Known variable metadata |
| `cli/src/env/identity.rs` | age identity generation and recipient inspection |
| `cli/src/env/upgrade.rs` | Reapply env templates to installed content |
| `cli/src/env/proxy.rs` | Transparent command proxy rules and execution |
| `cli/src/env/broker.rs`, `env/broker/{model,policy,store,commands}.rs` | Stable broker facade/DTOs, pure exact policy matching, private locked persistence and CLI commands; `env/mod.rs` owns reserved SSH keys |
| `cli/src/secret/mod.rs` | Backend-independent tagged ciphertext router |
| `cli/src/env/workspace/local_cache.rs` | Personal workspace hybrid preference and encrypted per-backend compilation cache; excluded from broker snapshots |
| `cli/src/secret/hybrid.rs` | Versioned authenticated GPG/age envelope and local selection |
| `cli/src/secret/gpg.rs` | GPG backend and exact hybrid public-recipient resolution |
| `cli/src/secret/age.rs` | age and Secure Enclave backend |
| `cli/src/secret/exec.rs` | Shared temporary-file and subprocess helpers |

## SSH

| Path | Responsibility |
|---|---|
| `cli/src/ssh/mod.rs` | System-ssh facade, forwarding resolution, session setup, child supervision and cleanup |
| `cli/src/ssh/{arguments,command}.rs` | Named parsed SSH operands, unchanged option-order/transport argv rules, and pure POSIX/encoded-PowerShell remote command construction |
| `cli/src/ssh/session_context.rs` | Trusted reconnect arguments and local session state |
| `cli/src/ssh/protocol.rs` | Transfer and log-relay wire frames |
| `cli/src/ssh/agent.rs` | Local listener and tracked connection-task lifetimes |
| `cli/src/ssh/agent/connection.rs` | Handshake, exact session-token checks, status and broker request routing; secret-response zeroization |
| `cli/src/ssh/agent/transfer.rs`, `transfer/{argv,relay}.rs` | Operand resolution/tool fallback/preview, pure trusted-context argv construction, and child/process-group supervision |
| `cli/src/ssh/remote_client.rs` | Remote-side `shine local` handlers |
| `cli/src/ssh/broker.rs` | Frozen session authorization, nonce claims, operation serialization and confirmation/decryption/enrollment orchestration |
| `cli/src/ssh/broker/interaction.rs` | Private interaction port, local TTY pause/restore/display/confirmation and owning confirmation guard; no policy or secret authority |

See [`data-flows.md`](data-flows.md#ssh-environment-forwarding),
[`invariants.md`](invariants.md#ssh-transfer), the
[transfer PRD](../../ssh-local-transfer-prd.md), and the
[secret-broker PRD](../../ssh-secret-broker-prd.md) before changing this subsystem.

## Other domains

| Path | Responsibility |
|---|---|
| `cli/src/task/mod.rs` | Personal command registry handlers and direct argv execution |
| `cli/src/task/manifest.rs` | `tasks.toml` model and persistence |
| `cli/src/theme/` | Terminal theme detection and synchronization |
| `cli/src/update_check/` | Release lookup, cache, notification, and binary upgrade |
| `cli/src/self_install.rs` | Stable handler facade for update, self-upgrade, local installation and configuration upgrade |
| `cli/src/self_install/update.rs` | Configuration/release update checks with their existing separate error policies |
| `cli/src/self_install/release.rs` | Release installation result rendering and best-effort local binary synchronization |
| `cli/src/self_install/binary.rs` | Local binary publication, destination persistence, synchronization and existing elevation policy |
| `cli/src/self_install/config_upgrade.rs` | Target/aggregate configuration upgrade routing, review, preparation, execution and rendering |
| `cli/src/self_install/config_upgrade/summary.rs` | Named target summary and domain-specific counters; display data carries no approval authority |
| `cli/src/home.rs` | Sudo-aware home discovery and path expansion |
| `cli/src/test_support.rs` | Shared process-state restoration guard and approved virtual Core installation fixtures |

## Embedded preset tree

| Path | Contents |
|---|---|
| `presets/shell/` | Shell commands/functions, including native and Bun runtimes |
| `presets/app/` | Managed application configuration, generators, hooks, and artifacts |
| `presets/sys/macos/` | macOS bootstrap manifest, scripts, and profile fragments |
| `presets/sys/ubuntu/` | Ubuntu bootstrap manifest, scripts, and profile fragments |
| `presets/sys/windows/` | Windows bootstrap manifest and PowerShell profile fragments |

Authoring rules and verification live in [`../preset-authoring.md`](../preset-authoring.md).
