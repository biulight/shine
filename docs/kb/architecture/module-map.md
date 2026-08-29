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
| `cli/build.rs` | `rust-embed` rebuild trigger for `presets/` |
| `core/` | Reusable `shine-core` package with no CLI/Tauri dependency |
| `core/src/lifecycle.rs` | Versioned frontend-neutral lifecycle result envelope and safe effect/status vocabulary |
| `core/src/plan.rs` | Versioned snapshot-bound security Plan, permission resolution, fingerprint, and approval contracts |
| `core/src/permission.rs` | Versioned target-local Preset permission declarations, normalization, and payload-free identity validation |
| `core/src/runtime/` | Internal Core runtime facade, immutable preset inputs, host ports, in-memory host, domain models, manifests, and migrated executors |
| `core/src/runtime/bootstrap.rs` | CLI/UI-shared, host-backed external/overlay discovery and immutable snapshot construction |
| `core/src/runtime/host.rs` | Observation-only filesystem/split-DNS ports plus inheriting filesystem, process, privileged, and system mutation ports |
| `core/src/runtime/planner.rs` | Workspace-internal pure App, Shell, and managed Sys Plan requests, snapshot capture, permission merge, and semantic-step generation |
| `core/src/runtime/app.rs` | Complete App assessment/install/upgrade/refresh/uninstall, generators, hooks, artifacts, embedded cache, and manifest orchestration |
| `core/src/runtime/shell.rs` | Complete Shell assessment/install/upgrade/uninstall/live render, launcher, cache, profile, and manifest orchestration |
| `core/src/runtime/sys.rs` | Managed Sys receipt assessment, managed-file/split-DNS orchestration, and run-manifest persistence |
| `core/src/runtime/sys_bootstrap.rs` | Sys v2 selection, preflight, detection, provider/script execution, post-detection, and batch persistence |
| `core/src/runtime/sys_profile/` | Sys profile composition, three-way reconciliation, phase sentinels, BOM and CRLF behavior |
| `core/src/runtime/validation.rs` | Host-backed preset discovery from a captured cwd, V1 diagnostics, and App/Shell/Sys schema validation |
| `core/src/runtime/inspection.rs` | Typed App/Shell inspection status and structural change vocabulary |
| `core/src/install/` | Core-owned transforms, EOL handling, host-required App manifest persistence, and host-neutral managed-file operations |
| `core/src/persist.rs` | Core-owned atomic persistence and versioned TOML helpers |
| `presets/` | Embedded shell, app, and OS bootstrap assets |
| `skills/shine-preset-author/` | Portable AI workflow and kind-specific preset author references |
| `docs/manual/` | Default English public manual |
| `website/i18n/zh-Hans/` | Simplified Chinese public manual and UI locale |
| `docs/kb/` | Internal agent knowledge: invariants, flows, decisions, operations, lessons |

## CLI entry and routing

| Path | Responsibility |
|---|---|
| `cli/src/main.rs` | `fn main`, top-level `run()` dispatch, background version notification |
| `cli/src/lib.rs` | Library module tree; exposes handlers that require lib-level tests |
| `cli/src/commands/cli.rs` | Root Clap parser and top-level argument types |
| `cli/src/commands/*.rs` | Domain subcommand enums and argument types |
| `cli/src/shim.rs` | Canonical `install`/`uninstall <TARGET>` routing and bare-category ambiguity checks |
| `cli/src/init.rs` | `shine init` project-local `shine.config.toml` creation |
| `cli/src/completion.rs` | Static and dynamic shell completion before ordinary CLI initialization |
| `cli/src/list.rs` | Top-level installed/available status views |
| `cli/src/info/` | Canonical target resolution, installed data collection, and diff rendering |
| `cli/src/path_display.rs` | Home-relative terminal path formatting |
| `cli/src/colors.rs` | Terminal color helpers |
| `cli/src/output.rs` | Shared command output mode and rendering support |
| `cli/src/presentation.rs` | CLI-private lifecycle events, writer-backed terminal renderer, and interaction ports |
| `cli/src/core_runtime.rs` | CLI settings and embedded-byte supply into the shared host-backed runtime bootstrap |
| `cli/src/platform.rs` | Platform classification shared across command domains |
| `cli/src/privilege.rs` | Cross-platform administrator/elevation orchestration |
| `cli/src/proc.rs` | Small domain-neutral subprocess helpers |
| `cli/src/persist.rs` | Compatibility re-export of Core persistence helpers |
| `cli/src/shell_quote.rs` | Copy-paste-safe shell argument rendering |
| `cli/src/version.rs` | Version string formatting |

### Command routing table

| Top-level command | Handler |
|---|---|
| `shell list/info/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `install/uninstall <TARGET>` | `cli/src/shim.rs` → `apps/` or `shells/` |
| `list [--available [KIND]]` | `cli/src/list.rs` or scoped list handlers |
| `info <TARGET>` | `cli/src/info/`; explicit system items delegate to `sys/` |
| `app artifact apply/remove <app-id>` | `cli/src/apps/build.rs` |
| `app refresh <app-id> [file]` | `cli/src/apps/refresh.rs` |
| `sys list/bootstrap/profile/...` | `cli/src/sys/` |
| `theme sync` | `cli/src/theme/` |
| `env ...` | `cli/src/env/` plus `cli/src/secret/` |
| `preset export/copy/link/unlink/overlay` | `cli/src/preset_commands.rs` |
| `preset new/validate` | Kind template handlers and `cli/src/preset_validation.rs` |
| `preset pull`, `update --pull`, `upgrade --pull` | `cli/src/git_pull.rs` plus top-level routing |
| `init` | `cli/src/init.rs` |
| `self install/upgrade` | `cli/src/self_install.rs`, `cli/src/update_check/` |
| `update [TARGET]`, `upgrade [TARGET]` | filtered app/shell/sys handlers plus update/self-install logic |
| `state migrate` | `cli/src/state.rs` |
| `serve install/start/status/uninstall/url` | `cli/src/serve.rs` |
| `ssh ...` | `cli/src/ssh/mod.rs` |
| `local download/upload/status` | `cli/src/ssh/remote_client.rs` |
| `task save/run/list/info/delete`, `run <NAME>` | `cli/src/task/` |
| `completions` | `cli/src/main.rs` with `clap_complete` |

## App installation

| Path | Responsibility |
|---|---|
| `cli/src/apps/mod.rs` | Shared app kernel and handler re-exports |
| `cli/src/apps/install.rs` | Core App-install request adapter and terminal rendering |
| `cli/src/apps/uninstall.rs` | Core App-uninstall request adapter and terminal rendering |
| `cli/src/apps/upgrade.rs` | Core App-upgrade request adapter and terminal rendering |
| `cli/src/apps/info.rs` | App list/info status |
| `cli/src/apps/report.rs` | Install/uninstall outcome formatting |
| `cli/src/apps/metadata.rs` | Compatibility re-exports of Core App metadata types |
| `cli/src/apps/annotation.rs` | Legacy `shine-dest:` annotation parsing |
| `cli/src/apps/refresh.rs` | Core explicit-generator-refresh adapter |
| `cli/src/apps/build.rs` | Core artifact apply/remove adapter |
| `cli/src/install_core/file_ops.rs` | Test-only compatibility coverage for Core host-backed copy, backup, and restore primitives |
| `cli/src/install_core/manifest.rs` | Compatibility re-export of Core-owned `app-manifest.toml` types |
| `core/src/install/` | App manifest, file ownership primitives, `jsonc-to-json`/`template`, and EOL helpers |

`install_core` contains app/sys-shared primitives only; `sys` depends on it, not on app-specific
logic.

## Shell deployment

| Path | Responsibility |
|---|---|
| `cli/src/shells/mod.rs` | Shell types, shared accessors, handler re-exports |
| `cli/src/shells/deployment.rs` | Hidden live-render Core adapter |
| `cli/src/shells/install.rs` | Core category/command install and upgrade adapter |
| `cli/src/shells/uninstall.rs` | Category/command uninstall results with sibling/cache and foreign-launcher protection |
| `cli/src/shells/links.rs` | Launcher/link specifications and conflict reporting |
| `cli/src/shells/report.rs` | Shell list/install/uninstall/upgrade reporting |
| `cli/src/shells/metadata.rs` | Compatibility re-exports of Core Shell metadata types |
| `cli/src/bun_runtime.rs` | Shared source-scoped Bun dependency policy and command construction |
| `cli/src/sentinel.rs` | Shared sentinel-block primitives used by shell and sys profiles |

## System configuration

| Path | Responsibility |
|---|---|
| `cli/src/sys/commands.rs` | Sys list/info/status rendering and Core bootstrap batch adapter |
| `cli/src/sys/detect.rs` | OS and Linux distribution detection |
| `cli/src/sys/managed.rs` | Managed-resource apply/update/remove/upgrade and structured result adapters |
| `cli/src/sys/model.rs` | Compatibility re-exports of Core Sys models |
| `cli/src/sys/manifest.rs` | Core parser compatibility adapter |
| `cli/src/sys/run_manifest.rs` | Compatibility re-export of Core-owned Sys manifest and receipt state |
| `cli/src/sys/selection.rs` | Selected-item terminal formatting only |
| `cli/src/sys/execution.rs` | Bootstrap reporting and proxy environment |
| `cli/src/sys/render.rs` | System command presentation helpers |
| `cli/src/sys/profile_commands.rs` | Core profile enable/disable adapter and rendering |

## Configuration, presets, and runtime state

| Path | Responsibility |
|---|---|
| `cli/src/config/mod.rs` | `Config` model and accessors |
| `cli/src/config/discovery.rs` | Global/project and environment path priority |
| `cli/src/config/load.rs` | Load/init and schema/version layering |
| `cli/src/config/save.rs` | Atomic, comment-preserving, sparse project saves |
| `cli/src/config/env_layer.rs` | `[env]` parsing/defaults and override files |
| `cli/src/presets.rs` | Embedded extraction, active asset reads, category enumeration |
| `cli/src/preset_commands.rs` | Preset copy/export/link/unlink/overlay commands |
| `cli/src/preset_meta.rs` | Test-only Core capability-report renderer for public manual parity |
| `cli/src/preset_validation.rs` | Core validation report text/JSON rendering and exit mapping |
| `cli/src/git_pull.rs` | FF-only external source pulls and managed overlay mirroring |
| `cli/src/state.rs` | Versioned runtime-state cleanup |
| `cli/src/status.rs` | Core App/Shell inspection-to-terminal row adapter |

Config discovery priority is documented as a behavioral contract in
[`data-flows.md`](data-flows.md#config-discovery) and
[ADR 0005](../decisions/0005-config-discovery-priority-chain.md).

## Environment and secrets

| Path | Responsibility |
|---|---|
| `cli/src/env/mod.rs` | `[env]` configuration and substitution core |
| `cli/src/env/commands.rs` | Env CRUD and secret command handlers |
| `cli/src/env/workspace.rs` | Workspace env sources, export, seal, and child execution |
| `cli/src/env/catalog.rs` | Known variable metadata |
| `cli/src/env/identity.rs` | age identity generation and recipient inspection |
| `cli/src/env/upgrade.rs` | Reapply env templates to installed content |
| `cli/src/env/proxy.rs` | Transparent command proxy rules and execution |
| `cli/src/env/broker.rs` | Secret-broker workspace snapshots and local policy store |
| `cli/src/secret/mod.rs` | Backend-independent tagged ciphertext router |
| `cli/src/secret/gpg.rs` | GPG backend |
| `cli/src/secret/age.rs` | age and Secure Enclave backend |
| `cli/src/secret/exec.rs` | Shared temporary-file and subprocess helpers |

## SSH

| Path | Responsibility |
|---|---|
| `cli/src/ssh/mod.rs` | System-ssh wrapper, session setup, remote command wrapper, cleanup |
| `cli/src/ssh/session_context.rs` | Trusted reconnect arguments and local session state |
| `cli/src/ssh/protocol.rs` | Transfer and log-relay wire frames |
| `cli/src/ssh/agent.rs` | Local transfer server and rsync/scp execution |
| `cli/src/ssh/remote_client.rs` | Remote-side `shine local` handlers |
| `cli/src/ssh/broker.rs` | Local authorization, TTY confirmation, and decrypt-on-demand broker |

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
| `cli/src/serve.rs` | Loopback server and per-user service management |
| `cli/src/update_check/` | Release lookup, cache, notification, and binary upgrade |
| `cli/src/self_install.rs` | Self-install and installed-configuration upgrade orchestration |
| `cli/src/home.rs` | Sudo-aware home discovery and path expansion |
| `cli/src/test_support.rs` | Shared cross-lib/bin test environment mutex |

## Embedded preset tree

| Path | Contents |
|---|---|
| `presets/shell/` | Shell commands/functions, including native and Bun runtimes |
| `presets/app/` | Managed application configuration, generators, hooks, and artifacts |
| `presets/sys/macos/` | macOS bootstrap manifest, scripts, and profile fragments |
| `presets/sys/ubuntu/` | Ubuntu bootstrap manifest, scripts, and profile fragments |
| `presets/sys/windows/` | Windows bootstrap manifest and PowerShell profile fragments |

Authoring rules and verification live in [`../preset-authoring.md`](../preset-authoring.md).
