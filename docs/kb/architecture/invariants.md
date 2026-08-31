# Invariants

Non-obvious properties that must hold. Breaking any of these has caused (or would cause) real
bugs. Check this list before changing the modules named in each entry.

## Install / uninstall safety

- **A security Plan is not dry-run and approval is snapshot-bound.** `PlanV1` contains only ordered
  semantic steps, safe diagnostic codes, resolved permissions, and digests of exact source/state
  observations. App, Shell, managed Sys, Sys bootstrap, App refresh/artifact, and Sys profile
  planners accept only observation host traits; they cannot write/remove, execute
  generator/hook/artifact/bootstrap/profile code, request privilege, or apply
  split-DNS state. Plans cannot carry content, env values, secret plaintext, raw argv, raw errors,
  or private source paths. Every supported App, Shell, managed Sys, Sys bootstrap, App
  refresh/artifact, and Sys profile mutation must enter through an approved Core method that
  regenerates the Plan from fresh captured inputs and matches both its
  fingerprint and exact required permission set before the first mutation; missing or uncomputable
  permissions fail closed. CLI `--yes` skips only the default-No prompt. Dry-run remains a separate
  preview contract. Specialized operations use `sys-bootstrap`, `app-refresh`,
  `app-artifact-apply/remove`, and `sys-profile-enable/disable`; they must not be described as
  lifecycle Plans or `LifecycleResultV1` operations.
- **Preset snapshot identity excludes checkout location but includes the trust layer.** The v1
  digest binds sorted effective logical paths, exact bytes, and each file's embedded, external, or
  overlay origin. It must not include physical source roots: relocating unchanged source is not a
  semantic change, while changing an effective layer is. State observations use the same framed
  SHA-256 contract. Labels use canonical targets and logical resources; request flags, relevant
  manifests/receipts, live fingerprints, platform/mode, and outcome-affecting input identities are
  all bound. Plain environment values contribute only a hash, while secrets require opaque handles
  or versions and never contribute plaintext.
- **A Preset permission declaration is not a grant.** App categories, Shell commands, and Sys items
  may declare schema-v1 capability identities, but those declarations do not create scoped
  external-code trust or bypass administrator authorization, ownership checks, or Plan approval.
  Filesystem declarations use logical bases and never embed a physical Preset
  checkout path; command entries contain no argv and environment entries contain names and
  sensitivity only. Pure planners merge explicit declarations with Core-bounded typed metadata and
  receipt ownership. A missing declaration or uncomputable requirement blocks protected mutation;
  it is never converted into a broad implicit grant.

- **Opaque Preset code is described conservatively, never sampled during planning.** A generator
  or lifecycle hook contributes known command/environment/administrator requirements plus an
  `execute` step and potential mutation step when its lifecycle trigger applies. Existing external-
  code gates may still block it. Embedded generator execution also declares and binds its runtime
  script materialization under the Shine directory. If the original Preset disappeared, supported
  manifests/receipts may drive owned-resource removal, but missing teardown code is never
  reconstructed or executed.
  User modification, occupied destinations, foreign launchers, and managed Sys ownership conflicts
  remain `preserve`/`blocked`; force must produce a distinct step or diagnostic and fingerprint.

- **App executable environment is explicit.** Generators receive only fixed `SHINE_APP_*` contract
  variables plus their `generator.env` mappings. Artifacts receive only the fixed contract plus
  their `[artifact].env` mappings, whose sources must be declared by the category's
  `[permissions].environment`. They never inherit the full active `[env]` table. Plain values are
  fingerprinted by hash and secret values require opaque versions; neither value form is serialized
  into the Plan.

- **Host observation is explicit and domain execution receives captured inputs.** Filesystem and
  split-DNS observation ports are separate from mutation ports, which inherit them; planner type
  bounds remain observation-only. A shared runtime
  bootstrap discovers external/overlay directories and constructs snapshots through the selected
  filesystem host, so CLI, UI, tests, and future host adapters reuse one implementation. Runtime
  contexts carry home, Shine roots, platform, environment, and the resulting immutable snapshot;
  App/Shell/Sys domain logic never rediscovers them from ambient globals. Distribution-only
  embedded bytes enter from the frontend. Validation additionally receives captured cwd, manifest
  persistence always requires a filesystem host, and Sys inspection/preflight stays read-only;
  only authorized script execution atomically materializes the category below the Shine runtime
  root. Core never reopens an external preset tree through `Path` helpers.
- **A host abstraction does not weaken ownership checks.** Real and in-memory managed-file paths
  share the same content hash, backup suffix, receipt, user-modification preservation, and
  manifest-version gates. Adding a new host operation must retain those checks rather than treating
  a successful write primitive as ownership evidence.

- **Reusable lifecycle results contain identities and codes, never payloads.** Structured outcomes
  may record canonical targets, logical resource names, status, effects, and stable diagnostic
  codes. They must not copy raw errors/logs, source or destination content, environment or secret
  values, subscription URLs, credentials, or absolute destination paths into a reusable result.
  Human CLI diagnostics remain subject to their domain-specific redaction rules.
- **Lifecycle presentation is a CLI-only side channel, never a result payload.** App, Shell, and
  managed Sys may emit home-relative paths, human diagnostics, hook notes, and section events to a
  replaceable reporter, but those events are not serializable and must never be copied into
  `LifecycleResultV1`. Confirmation and administrator authorization use the separate interaction
  port so reusable execution does not depend on dialoguer or terminal rendering.
- **Lifecycle status distinguishes observation from execution.** Read-only `update` uses
  `dry_run = false` and `Pending`; explicit dry-run uses `dry_run = true` and `Previewed`.
  `Changed` means this execution changed Shine-owned state. App hooks/teardown may record execution
  effects without changing their established non-fatal exit semantics; managed Sys user
  modification remains a typed preservation conflict while the CLI keeps its existing failure exit.
- **App update presentation and reusable results share one Core assessment pass.** `AppRow` and
  `LifecycleOutcomeV1` must derive from the same per-file assessment; do not rebuild rows to obtain
  the structured result. Default read-oriented assessment never executes generators and reports
  `app_generator_not_evaluated` when dynamic desired content needs explicit execution.
  `--run-generators` is a separate opt-in assessment mode: it may execute selected generators once
  to derive in-memory desired content but must not write destinations/manifests or run hooks and
  artifacts. Typed inspection paths remain non-serializable and must not enter the lifecycle result.
- **Managed-file update details are field labels, not payloads.** The read-only comparison may
  report that destination or content changed, but must not copy the destination, desired bytes, or
  environment values into structured lifecycle outcomes. Ownership and user-modification checks
  still occur at apply/remove time against the receipt and live resource.
- **Every runtime manifest is a pre-mutation gate.** App, Shell, and Sys load their own manifest
  before destination/resource, embedded cache, snapshot/render, launcher, receipt, or profile
  mutation. Missing `schema_version` is legacy v0, read-only loads never rewrite it, the next
  successful mutation writes v1, and an unsupported future version fails first. The inner Sys
  receipt `version` is independent of `sys-manifest.toml`'s schema version.

- **Uninstall never touches user files.** `presets::remove_prefix` removes only embedded-asset
  files; `bin_links::unlink_managed` removes only symlinks pointing into the managed presets dir,
  plus **regular files that carry the `# shine-managed` marker and a `# shine-target:` under the
  managed root** (Windows `.ps1`/`.cmd` shims and Unix `runtime = "bun"` launcher scripts); app
  uninstall is driven by `~/.shine/app-manifest.toml` entries only.
- **Bun launchers are marked regular files, identified only by content, never by name.** A Unix
  `runtime = "bun"` command is a generated executable script (not a symlink); ownership and
  current-ness (`bin_links::launcher_target`/`unix_launcher_status`, mirrored by
  `windows_shim_status`) key on the `# shine-managed` marker + a `# shine-target:` path under the
  managed root. An unreadable/non-UTF-8/unmarked file is always `NotManaged` — treated as a user
  conflict, never overwritten or removed. `unix_bun_launcher_content` is byte-deterministic so a
  format change re-detects installed launchers as stale on upgrade; changing it is a format bump.
  The content embeds the entry's ordered `env` spec (the `--with` tokens of the
  `shine env run --no-workspace … -- bun … <script>` wrapper) and its Bun dependency policy, so
  either declaration changing refreshes the launcher. Ownership/removal still key only on the
  marker + target, independent of `env` and dependency mode.
- **Backups use the `<name>.shine.bak` suffix** (`install_core/file_ops.rs::backup_path`).
  Uninstall restores from that exact name; changing the suffix orphans existing backups.
- **An app source has exactly one manifest destination.** A per-file `dest` overrides the category
  root, but changing that effective destination must relocate by manifest `source`: verify the old
  copy is unmodified and the new path is free, then replace the manifest entry. Never update the old
  path and separately install the new one; that leaves two owned copies for one source. Duplicate
  effective destinations within the active metadata fail before any install or upgrade writes.
- **`requires_admin` must persist on every manifest entry** (`install_core/manifest.rs::AppEntry`).
  Uninstall routes to the sudo removal path based on the stored flag, not by re-checking the
  path. Dropping it during (de)serialization silently breaks privileged uninstall (commit
  `70ee910`).
- **Privileged filesystem transactions must hold the host-provided cross-process admin lock** for
  their complete ownership-check, backup, mutation, and rollback sequence
  (`PrivilegedFileSystemHost::acquire_privileged_operation`, `$TMPDIR/shine-admin.lock`). Locking
  individual writes is insufficient because another process could race between backup and replace.
  Self-install uses the same host lock (commit `fbd9c55`).
- **No code path should ask the user to manually type `sudo` on Unix.** Frontend interaction
  authorizes through `privilege::ensure_admin`; the real privileged host performs non-interactive
  mutations with the cached authorization. Self-install uses its CLI-only command adapter.
  Windows has no `sudo` equivalent, so its privileged paths still surface a manual
  "rerun elevated" hint instead.

## Declarative actions and recovery

- **The executable Action IR is not the security Plan.** `PlanV1` remains a payload-free review and
  approval contract. `ActionIrV1` is created for execution only after planning and may carry resolved
  effect paths and content hashes, but never managed bytes, environment values, secret plaintext, or
  raw argv. Do not embed the Action IR in `PlanV1` or reinterpret semantic Plan steps as executable
  instructions.
- **Recovery is an explicit, freshly approved operation.** Ordinary planning, status, install,
  upgrade, and uninstall must not mutate an interrupted journal implicitly. `app-recovery` binds the
  exact journal bytes and current destination observation, validates its approval again under the
  host-provided cross-process operation lock, and only then mutates recovery state. The CLI exposes
  this only as `shine app recover [--yes]`; a ready Plan must show journal removal and require
  default-No approval, while blocked recovery preserves the journal. Background release gating must
  not make this recovery entry point unavailable.
- **A transaction-created file is rollback-owned only while unchanged.** The v1 App creation slice
  may remove a destination only when its bytes still match the Action IR's desired hash. Missing is
  safe journal cleanup; any other content is a blocking user modification and both destination and
  journal remain for explicit resolution.
- **A transaction-created App backup is restorable only while both regular-file paths match.**
  Backup-aware v1 creation binds the fixed `.shine.bak` path plus original and desired hashes, and
  starts only from an unowned regular file when that backup path is absent. Recovery may restore
  only from `(missing, original)` or `(desired, original)` destination/backup state;
  `(original, missing)` means mutation never started. Every other combination blocks and preserves
  destination, backup, and journal. The journal never stores either byte payload, a matching durable
  receipt commits ownership of both paths, and any non-matching receipt that claims the source or
  either path blocks recovery.
- **A managed App update moves, never serializes, its previous bytes.** The Phase 4 in-place update
  slice applies only to an unchanged receipt-owned static Copy at the same
  destination. Before replacement it journals, then renames the previous file to the canonical
  same-directory `<name>.shine.rollback` path; that path must be absent and unclaimed. The Action IR
  binds the previous App backup identity, prior mode and original/desired hashes but never either
  byte payload. Apply restores the prior mode on the replacement. Before the replacement receipt is
  durable, recovery restores only the exact original/missing, missing/original, or desired/original
  state; after it is durable, commit/recovery removes only unchanged rollback material. Any changed
  kind, bytes, mode-bound input, receipt, destination or rollback path blocks and preserves state.
- **An ordinary managed App removal commits through receipt absence.** The Phase 4 removal slice
  applies only to an unchanged, receipt-owned static Copy with no persistent backup and no force.
  It moves the destination to the canonical same-directory rollback path before
  removing the receipt. While the exact old receipt remains, recovery restores only an unchanged
  regular rollback file with the recorded mode and hash. After receipt removal, the journal must
  durably enter `receipt-committed` before commit/recovery removes unchanged rollback material; bare
  receipt absence cannot authorize cleanup and instead makes explicit recovery reconstruct the exact
  old receipt before restoring unchanged bytes. No conflicting manifest receipt may claim the action
  source, destination or rollback path. Any
  occupied destination, changed path, receipt conflict, backup, JSON merge or force
  case stays outside this action and must not inherit its rollback proof.
- **A backup-restoring App removal binds both file identities across two moves.** The applicable
  static Copy receipt must own the canonical `.shine.bak`, and the managed destination,
  persistent backup, and canonical `.shine.rollback` must be unchanged regular-file/missing states
  matching the Action IR's two modes and hashes. Before receipt commit, recovery accepts only exact
  managed/original/missing, missing/original/managed, or original/missing/managed
  destination/backup/rollback states; it reconstructs a missing exact old receipt before returning
  the user file to backup and managed rollback to destination. After `receipt-committed`, recovery
  keeps the exact restored user destination and removes only unchanged managed rollback material.
  Any changed kind, mode, hash, receipt, or claimed path blocks and preserves all three paths.
- **A forced App removal binds the modified file separately from its receipt.** The applicable
  Phase 4 action is limited to a receipt-owned static Copy regular file whose current
  hash differs from the receipt hash. The Plan must carry the explicit user-modification override.
  The journal stores both hashes and the current mode, but never file bytes, before moving the
  modified destination to canonical same-directory rollback material and optionally restoring the
  exact fixed backup. Before receipt commit, recovery reconstructs a missing old receipt and
  restores the exact modified destination plus optional backup; after `receipt-committed`, it keeps
  the completed uninstall state and removes only unchanged modified rollback material. Any changed
  kind, mode, hash, receipt, destination, backup, or rollback path blocks. Unchanged destinations
  use the ordinary removal action even under `--force`; non-static-Copy paths do not inherit this
  proof.
- **App upgrade stale pruning is receipt removal, not a generic upgrade write.** A stale static
  Copy or JSON entry may reuse the corresponding removal Action only when its current owned state
  still matches the receipt and the approved Upgrade Plan carries `app_stale_source_pruned` for the
  exact target/resource. Planning must bind destination, optional persistent backup, canonical
  rollback material, manifest and journal effects with removal permissions even though the outer
  lifecycle operation is Upgrade. User-modified stale state is preserved, forced removal is not
  inferred, and a missing destination performs receipt-only cleanup without administrator access.
  Receipt absence still requires the positive journal commit marker before rollback material may
  be discarded.
- **App static Copy relocation is one receipt replacement, not create plus remove.** An approved
  `RelocateManagedFile` binds the exact old receipt, old destination, optional canonical persistent
  backup, old same-directory rollback, absent new destination, desired hash, and both privilege
  identities. The journal precedes staging the old managed file, restoring its backup, and writing
  the new file. Before the new receipt is durable, recovery removes only an unchanged new file and
  restores the exact old path/backup state; after it is durable, recovery preserves the final paths
  and removes only exact rollback material. The new receipt never inherits the old backup path. A
  missing old destination is eligible only without a persistent backup; JSON relocation does not
  inherit this whole-file proof.
- **Privileged App static Copy changes the mutation port, not the rollback proof.** Create,
  backup-aware create, update, and the three removal actions persist `requires_admin`, derive
  Administrator permission, and require matching old/new receipts to carry the same flag. Planning
  requests elevation only for an actual protected path mutation. Apply, receipt commit, and recovery
  hold the host-provided cross-process administrator lock across revalidation and use privileged
  write/move/remove/mode operations for destination, persistent backup, and transaction rollback
  paths. A freshly reviewed recovery Plan requests Administrator only when its exact safe state
  changes one of those paths; receipt-only repair and journal cleanup do not. CLI recovery obtains
  authorization after Plan approval and before mutation.
- **App JSON merge owns declared keys, never the whole object.** Install/update and ordinary or
  forced removal bind the exact pre-operation JSON file as same-directory rollback material, but
  recovery reads it only to restore declared unique top-level keys into the current object. It must
  preserve every current unrelated value. Creation at an absent path removes the whole file only
  when no unrelated keys exist; after removal receipt commit, current JSON is user-owned and only
  unchanged rollback material may be removed. Invalid JSON, changed managed keys, changed rollback
  kind/hash/mode, or a receipt conflict blocks without mutation. Prior and desired JSON values never
  enter Action IR or the journal.
- **App JSON relocation owns keys independently at both destinations.** `RelocateManagedJson`
  binds the exact old/new receipt identities, separate old/new managed-key sets and subset hashes,
  the optional old whole-file identity, canonical old rollback path, and an absent new destination.
  Before the new receipt is durable, recovery removes only the desired keys from the new object and
  restores only the previous keys into the old object, preserving unrelated current values on both
  sides. After receipt commit, the old object is user-owned; recovery preserves it, verifies the new
  managed subset, and removes only exact rollback material. Missing old state is supported without
  rollback. Invalid JSON, changed managed keys, rollback changes, occupied paths, or receipt
  conflicts block all recovery mutation. Neither prior nor desired JSON values enter Action IR or
  the journal.
- **The journal precedes mutation and outlives the receipt.** Write the versioned journal before the
  first action mutation, update action state atomically, persist the matching domain receipt, and
  only then commit by removing the journal. An existing or unsupported-version journal blocks a new
  operation; it is never overwritten, upgraded, or discarded best-effort. App journal commit must
  re-read and match the durable manifest receipt. If interruption happens after that receipt is
  durable, explicit recovery preserves the now manifest-owned resource and removes only the stale
  journal.
- **A first-time Shell launcher is rollback-owned only while every created resource is exact.**
  `CreateShellLauncher` applies only when the command has no receipt and all launcher paths are
  absent. One action covers a Unix symlink, a generated Unix Bun/live file, or both Windows shim
  files. `shell-operation-journal.toml` precedes the first path mutation and survives until the
  complete command receipt is durable. Explicit `shell-recovery` removes only exact
  target/hash/mode matches when that receipt is absent; any changed resource or conflicting receipt
  blocks, while an exact durable receipt preserves the launcher and authorizes stale-journal
  cleanup. Profile sentinel blocks, shared snapshots, launcher updates, and removals use separate
  proofs.
- **A receipt-owned Shell launcher update moves exact old resources before replacement.**
  `UpdateShellLauncher` applies only when the old command receipt is still exact, the current Unix
  launcher or every Windows shim matches the launcher deterministically reconstructed from that
  receipt, and each canonical same-directory `.shine.rollback` path is absent. The journal binds
  complete old/new receipts plus target/hash/mode identities, never launcher bytes. Each changed
  old resource moves to rollback before its replacement is written. While the old receipt remains,
  recovery restores only exact missing-or-replaced states; after the new receipt is durable,
  commit/recovery keeps exact replacements and removes only unchanged rollback material. Any
  changed resource, rollback path, or conflicting receipt blocks and preserves all paths. Foreign,
  already modified, removal, shared snapshot/render, and profile-block paths do not inherit this
  update proof.
- **A receipt-owned Shell launcher removal commits through a positive journal marker.**
  `RemoveShellLauncher` applies only when the exact old command receipt still exists, every Unix
  launcher or Windows shim resource deterministically reconstructed from it remains exact, and all
  canonical same-directory `.shine.rollback` paths are absent. The journal precedes moves of every
  launcher resource to rollback. After manifest receipt removal, the journal must durably record
  `receipt-committed` before commit or recovery may clean exact rollback material; bare receipt
  absence instead requires explicit recovery to reconstruct the complete old receipt before
  restoring exact resources. Any conflicting receipt, changed destination or rollback identity, or
  occupied rollback path blocks and preserves state. Foreign and modified launchers do not inherit
  this proof; shared snapshot/render state and profile sentinel blocks remain separate actions.
- **An external Shell snapshot is category-owned, and receipt presence is not its commit marker.**
  `ReplaceShellSnapshot` applies only to approved snapshot-mode selections whose selected commands
  require no rendering. It binds the whole sorted category tree, deterministic sibling stage and
  rollback directories, and all selected command receipt transitions. The journal precedes staging;
  the old tree remains exact rollback material until the selected receipts and a positive
  per-action commit marker are durable. Before that marker, recovery must evaluate launcher actions
  against the previous receipt set and restore the previous tree; afterward it may remove only the
  exact old rollback tree. Extra or changed stage files, changed destination/rollback trees, or
  conflicting receipts block all recovery. Embedded cache, rendered outputs, uninstall, and profile
  sentinel blocks do not inherit this proof.
- **Opaque execution is never granted declarative rollback by classification alone.** Hooks,
  generators, artifacts, shell bodies, scripts, and package providers retain explicit provenance,
  privilege, permission and unsupported-rollback classification until a narrower typed action
  replaces them. Keep `executable-preset-inventory.md` current when built-in executable capability
  changes.

## Shell profile editing

- **Sentinel blocks are the only thing shine writes to user shell configs**
  (`# >>> shine >>>` … `# <<< shine <<<`, Core Shell profile handling; sys uses per-phase sentinels
  like `# >>> shine <os> sys pre >>>`). Both delegate to the shared primitives in
  `core/src/sentinel.rs` (`find_block`/`extract_block_with_newline`/`remove_block_bytewise`/
  `remove_block_linewise`/`insert_block`/`trim_outer_blank_lines`).
- **Two sentinel removal styles exist and must not be unified without golden-output proof.**
  `sentinel::remove_block_bytewise` (shells' semantics) consumes one preceding blank line and
  never rewrites line endings; `sentinel::remove_block_linewise` (sys' semantics) never consumes
  a preceding blank line but normalizes CRLF to LF unconditionally (via `str::lines`), even when
  the sentinel isn't present. Canonicalizing them without characterization tests proving neither
  caller depends on the difference risks a silent formatting regression in a file shine doesn't
  own.
- **Line-ending differences must not register as changes on the sys path.** Preset templates are
  pinned LF (repo `.gitattributes`: `presets/** text eol=lf`), so the rust-embed'd template is
  byte-deterministic across build hosts. Core Sys reconciliation (`runtime/sys_profile/`) compares
  content line-ending-agnostically via `install::eol_eq`/`normalize_eol`: a pure CRLF↔LF difference
  (e.g. a Windows editor
  re-saving an installed loader file or profile) reports **no update** and leaves the user's file
  bytes untouched. When on-disk endings differ, the three-way merge skips `git merge-file` (which
  would see spurious per-line diffs) and uses the pure-Rust fallback over normalized bytes.
  Genuine content changes still write LF. The two sentinel *removal* styles above are unchanged —
  only the comparison layer normalizes.
- **Paths under `$HOME` are written as `$HOME/...`**, not absolute, for portability.
- **PowerShell profiles: preserve a leading BOM** when rewriting the file
  (`core/src/runtime/sys_profile/blocks.rs`, commit `81244f8`), and update **both** `Documents/PowerShell/` and
  `Documents/WindowsPowerShell/` profile files so pwsh and Windows PowerShell stay in sync.
- **Sys profile composition is activation-additive, not selection-replacing.** A targeted item or
  named selection profile enables successful item integrations but never disables previously
  enabled ones. Only `shine sys profile disable <ITEM>` removes that item's generated content.
  Composition order is phase, explicit priority, manifest order, then integration order; failures
  must leave the last installed profile intact.

## Config files

- **All `config.toml` writes go through `shine_core::sync_table`**, which preserves user comments.
  Never serialize the whole file from a struct — that destroys comments.
- **Config discovery priority is fixed**: `SHINE_CONFIG_DIR` > `SHINE_PRESETS` > `presets_dir`
  key > `~/.shine/` default. Code and
  [`data-flows.md`](data-flows.md#config-discovery) must agree.
- **External app preset hooks and generators require scoped trust.** `post_upgrade`
  runs commands after upgrades, while an automatic file generator may run during an approved
  install/upgrade and supply effective source bytes. Embedded code may run implicitly, but external
  preset or overlay code requires a grant matching canonical target, capability, code digest,
  trust layer, and exact permission set. Read-oriented checks execute it only through the explicit
  `--run-generators` mode; ordinary inspection remains process-free.
- **Bun package installation is source-scoped and explicit.** Embedded scripts and external scripts
  without a locked declaration run with `--no-install`. Only an effective external/overlay script
  whose own physical category contains both `package.json` and `bun.lock` may run with
  `--install=fallback`; one file without the other and any `trustedDependencies` field are errors.
  Overlay package metadata never changes an inherited embedded script. Shine never runs
  `bun install`, owns `node_modules`, or cleans Bun's global cache/virtual store, and dependency
  download never bypasses scoped trust. See ADR 0031 and ADR 0046.
- **External sys executable code requires target-local scoped trust.** Static detection/provider metadata and
  declarative PATH/env/aliases are safe to inspect, but external or overlay bootstrap/managed scripts,
  guarded eval/source, fragments, and base profile code require a matching `sys/<item>` grant.
  Read-only status paths must never execute sys code. Project config and Presets cannot authorize
  their own executable content.
- **Manual generators never run from implicit status or upgrade paths.**
  `generator.auto = false` leaves ordinary `list`/`info`/`update` local-only and causes upgrade to
  preserve the manifest snapshot. Install, `shine app refresh`, or explicit
  `info`/`update --run-generators` evaluation may run it. Evaluation never writes; refresh must
  target manifest-owned files and preserve user modifications unless `--force` is explicit.
- **Generator failures never destroy the last-known-good managed file.** Status
  and upgrade warn and retain an existing manifest-owned destination. An enabled
  generator with no successful installed snapshot fails rather than installing
  empty or partial output. Generator diagnostics must not include source URLs,
  credentials, or raw subscription records.
- **The Surge profile artifact treats `SURGE_PROFILE` as a user-owned file.**
  `profile-artifact.ts` rejects symlinks and invalid UTF-8, computes the full
  desired content before writing, replaces through a same-directory temporary
  file, and preserves mode and per-line endings. A missing patchable
  `#!include` is an error, never a successful no-op. Keep parsing and filesystem
  behavior shared by build and teardown; do not duplicate it in an overlay.
- **Local HTTP resources share one loopback server.** Files that need stable local URLs live under
  `<shine_dir>/http/` and are served by `shine serve start`; `shine serve install` registers one
  global user service for that server. Do not add per-app HTTP daemons, ports, or launchd jobs.

## Personal tasks

- **`tasks.toml` lives under `Config::shine_dir()`**, so it follows `SHINE_CONFIG_DIR` for free.
  Never resolve it against `presets_dir` or `$HOME` directly — that would break test isolation.
- **`shine task run` propagates the child exit code verbatim** and never runs the saved argv
  through a shell. Wrapping the failure in an anyhow error (or defaulting to exit 1) would corrupt
  the task's own exit semantics. Shell syntax is opt-in via an explicit saved `sh -c '...'`.
- **A missing task `cwd` means dynamic caller cwd.** Legacy `tasks.toml` entries have no `cwd`, so
  deserialization must default it to `None`; only an explicit `task save --cwd` fixes the working
  directory. Never reinterpret missing `cwd` as the save-time directory.

## Embedded presets

- **`cli/build.rs` must keep `cargo:rerun-if-changed=presets`.** Without it, preset edits
  don't trigger re-embedding and the binary silently ships stale assets.
- **The generated built-in platform capability blocks must use runtime selector semantics.**
  `preset_meta::tests::built_in_preset_platform_capability_docs_are_current` asks Core to derive App
  category and Shell command visibility from the pristine immutable snapshot for macOS, Linux, and Windows,
  then checks the delimited blocks in both public manual locales. Do not maintain a parallel
  platform map or weaken the test to trust prose labels; a preset metadata change and both generated
  blocks must land together. Regenerate them with
  `SHINE_UPDATE_PRESET_CAPABILITIES=1 cargo test built_in_preset_platform_capability_docs_are_current`.
- **Fallback depends on the selected preset mode.** In built-in mode, an overlay replaces matching
  paths and unmatched paths continue to read embedded assets. A full external preset source is
  authoritative for app and shell category discovery: a missing category or file is not silently
  borrowed from the binary. Sys profile installation has a narrower compatibility fallback to its
  embedded template when a selected external sys preset omits a previously-known profile file
  (commit `5606438`). Do not generalize that compatibility path into cross-source category fallback.

## External shell deployment

- **Shell command activation is command-scoped even though deployment is category-scoped.**
  Embedded extraction and external snapshots may materialize every source file in a category so
  commands can use sibling resources, but source-file presence alone never means that a command is
  installed. `shell-manifest.toml` entries and compatible legacy launchers are the activation
  receipts. Command-scoped install must upsert only its selected receipt; command-scoped uninstall
  must remove only its selected managed launcher/receipt and preserve installed siblings.
- **External source selection and installed state are separate.** Snapshot mode materializes
  effective shell categories below `<shine_dir>/installed/shell/`; launchers must never point at
  the user-owned external tree unless `external_shell_mode = "live"` is explicit.
- **Live transforms are manifest-constrained.** Generated launchers may request only a canonical
  target recorded in `shell-manifest.toml`; the renderer writes only below `rendered_dir`, stores
  no env values in the manifest, uses atomic replacement, and fails rather than executing stale
  output after a transform error.
- **External uninstall never deletes source.** It may remove Shine-owned snapshots, rendered
  files, manifest entries, and managed launchers, including legacy launchers pointing into the
  external tree. The external presets and overlay directories remain untouched.
- **Shell update/upgrade must preserve foreign launchers.** Ownership is checked with the same
  managed-root proof used by uninstall. A regular launcher outside that proof is a structured
  `Conflict`, is not counted as a pending update, is excluded from forced launcher refresh, and
  retains its command receipt. Compatible legacy/stale symlinks keep their existing upgrade repair
  behavior.
- **Preset materialization excludes `node_modules/` at every depth.** External Shell snapshots and
  overlay copies retain `package.json` and `bun.lock`, but never copy a local installation tree into
  Shine-owned state or embedded extraction.

## Secrets

- **Decrypt routing is tag-based only** (`secret::parse_tagged_ciphertext`). `decrypt_secret`
  must never consult `Config::secret_backend` or any other config to pick a backend — only the
  `age:` prefix (or its absence) decides. This lets `secret_backend`/`age_recipients` change
  freely without breaking previously-encrypted secrets (see
  [ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md)).
- **GPG ciphertext stays untagged.** Adding a tag to existing GPG secrets, or changing the `age:`
  prefix, breaks every secret encrypted before the change.
- **Workspace export decrypts only on explicit request.** `shine env workspace export` omits
  secret-winning keys unless `--include-secrets` is present, never mixes in the caller's process
  environment, and never prints exported values in its status or dry-run output. On Unix, an
  export containing secrets must be created privately (`0600`) before plaintext bytes are written;
  chmod after a wider temporary-file write is not sufficient.

## SSH transfer

- **`ssh::agent` must not trust wire-supplied fields beyond the session token.** The token is the
  only authorization check on a `PutFile`/`GetFile`/`Status` request, but it travels to the remote
  host as plain argv/environ (`env SHINE_SSH_TOKEN=...` in `ssh::mod`), so it can leak to other
  local users there via `ps eww`. Any field documented as constrained (e.g. `PutFile.filename` is
  meant to always be a bare basename) must be validated as such where it's consumed
  (`agent::ensure_single_path_component`), not just produced correctly by the one trusted
  `remote_client` implementation. `dest_hint`/`source_hint` are expanded with `~`-only
  substitution (`home::tilde_expand`), never the full `${VAR}` expansion used for locally-typed
  paths elsewhere, so a forged hint can't pull values out of the local agent's own environment.
- **Per-connection transfer tasks must stay tracked in `agent::ConnectionTasks`, not bare
  `tokio::spawn`.** `agent_handle` (the accept loop's `JoinHandle`) does not cover them —
  `agent_handle.abort()` only stops new connections, never an in-flight transfer. `handle_ssh`
  must drain `ConnectionTasks` before removing the session directory or exiting, so a still-running
  transfer's own error-path cleanup gets to finish instead of being cut off by process exit.

## SSH secret broker

- **The session token is transport correlation, not broker authorization.** A remote peer that
  learns `SHINE_SSH_TOKEN` still must satisfy the session's exact direct allow-list or an exact
  local workspace policy. Every broker wire field is bounded, rejects control characters, and is
  treated as untrusted input.
- **Direct requests always require a local TTY confirmation and decrypt only `KEY_SECRET`.** They
  never use the plaintext fallback accepted by ordinary `env run`, and
  `--trust-remote-session` never suppresses direct-request confirmation.
- **Workspace authorization binds the whole request.** Matching includes SSH target, workspace
  bytes/hash, every source path and byte hash, mode, complete declared-secret set, exact release
  mapping, and exact argv. The local agent must reject rather than partially match any difference.
- **All-declared release is expansion, never a wildcard.** `--release-all-declared` resolves the
  current immutable snapshot into a sorted explicit release list before policy creation or update.
  A future declared secret/source change must invalidate the old policy and require review; runtime
  matching must never reinterpret the stored list as “whatever exists now.”
- **Workspace files are read once per broker run.** The remote snapshot sent for authorization and
  the values later merged into the child environment derive from the same in-memory bytes; never
  re-open a source after the local agent approves its hash.
- **Broker policies are local security state.** `<shine_dir>/ssh-secret-broker.toml` must not be a
  symlink, must be owned by the current Unix user, and must have mode `0600`; writes use an atomic
  same-directory temporary file. Remote enrollment is allowed only in the explicit trusted mode
  and still requires local confirmation.
- **Broker operations are serialized per SSH session.** A local confirmation keeps the SSH child
  paused and the TTY in its pre-SSH canonical/echo state through the subsequent GPG/age operation,
  then an RAII guard restores raw termios and resumes SSH on success or every error path. Dropping
  the guard immediately after the yes/no prompt breaks terminal pinentry and permits concurrent
  requests to corrupt each other's TTY interaction.
- **Broker UI is rendered only inside the local-TTY guard.** OpenSSH raw mode disables normal LF
  cursor handling, so inspect/enrollment details printed before termios restoration form a
  staircase and can interleave with the remote shell. Confirmation accepts plain `y`/`yes` and
  those exact values inside the standard bracketed-paste wrapper; arbitrary ANSI/control-decorated
  input remains a rejection.

## Local HTTP server

- **`serve::handle_start`/`handle_install` have no authentication of their own.** Binding
  loopback-only (`127.0.0.1`) keeps the server off the network, but it does not stop other local
  OS user accounts on a shared/multi-user machine from connecting and reading any file under
  `serve::http_root()` (`~/.shine/http/`), bypassing the filesystem permissions that would
  otherwise keep them out of this user's home directory. Preset authors must never route secrets
  or other sensitive content through a `dest` that resolves under `~/.shine/http`.
- **Persistent serve registrations are per-user and preserve the selected Shine directory.**
  launchd, systemd, and Windows Task Scheduler entries must run without administrator privileges
  and pass the resolved `shine_dir` back through `--config-dir`; background startup cannot depend on
  the installing shell retaining `SHINE_CONFIG_DIR` or its current directory.
- **launchd log paths must stay under the user's own `shine_dir`, never a shared path like
  `/tmp`.** `serve::launchd_log_dir` writes to `shine_dir/run/http/serve.{out,err}.log`, kept out
  of `http_root()` itself so log contents are never servable over HTTP. Two OS user accounts each
  running `shine serve install` would otherwise collide on the same fixed `/tmp/<label>.log` path.

## Update check

- **A failed or rate-limited version check must never fail the user's command** (`main.rs`,
  commits `605fdd8`, `f033a25`). Network errors are tolerated; rate-limit cooldowns are cached.
- **`preview` is not a release baseline.** Version comparisons and release notes must use the
  latest stable `v*` tag (see `conventions.md` § Versioning).
- **A targeted `shine upgrade <TARGET>` must not mutate other targets.** App filtering happens
  before stale-entry handling, shell filtering happens before link/template reconciliation, and
  managed sys apply receives the selected item. Filtering only the report after a global upgrade
  would violate the command's user-visible scope.

## Tests

- **Env-var mutation in tests must hold `crate::test_support::env_lock()`** — a single shared
  mutex used across the lib/bin boundary (it is deliberately *not* `#[cfg(test)]`-gated because
  `cfg(test)` does not cross that boundary).
- **Tests that touch real system paths** (e.g. docker-engine's `/etc/docker/daemon.json`) must
  additionally hold the cross-process admin lock for their full body (commit `fbd9c55`).
