# Shine Security Plan and Trust Model PRD

> **Status:** Roadmap Phase 3 contract foundation, permission declarations, and pure App/Shell/
> managed Sys planners implemented; CLI enforcement and coarse-grant migration remain future
> slices. This document is internal and does not define released CLI or JSON behavior.

## Summary

Roadmap Phases 1 and 2 established versioned lifecycle results, immutable Preset snapshots,
captured runtime context, Core-owned App/Shell/Sys execution, and real/in-memory host ports. Phase
3 adds a separate security boundary: every Preset lifecycle mutation must eventually be preceded by
a reviewable Plan that derives all required permissions and is bound to the exact source and state
inputs later applied.

The first slice added the versioned Plan, permission, snapshot digest, fingerprint, and approval
contracts to `shine-core`. The second added target-local permission declarations, static
validation, and built-in migration. The third added workspace-internal pure planners for App,
Shell, and managed Sys. None routes current commands through a security Plan; existing dry-run,
status, external-code gates, and mutation paths remain compatibility behavior.

## Goals

1. Represent reviewable lifecycle intent without embedding executable actions or private payloads.
2. Cover filesystem, network, command, administrator, environment/secret, and system permissions.
3. Fail closed when a required permission is undeclared or cannot be computed.
4. Bind approval to exact Preset and observed-state snapshots, ordered semantic steps, and the
   complete required permission set.
5. Reject apply after any source, state, step, or permission change; permission expansion always
   requires a new review.
6. Keep planning free of host mutation, Preset code execution, and secret plaintext.

## Non-goals of the delivered planner slice

- No bootstrap, App artifact/refresh, or Sys profile planner.
- No CLI `plan` command, prompt, JSON output, or mutation enforcement.
- No Declarative Action IR, journal, rollback, or recovery; those remain Roadmap Phase 4.
- No change to `allow_app_hooks`, `allow_sys_code`, or current external-code behavior.

## Contract v1

`PlanV1` covers the lifecycle operations already represented by `LifecycleOperation`: install,
update, upgrade, and uninstall for App, Shell, and managed Sys. Bootstrap, artifact, refresh, and
profile operations require a later operation-contract slice before enforcement.

A Plan records:

- `schema_version = 1` and the lifecycle operation;
- one SHA-256 digest for the effective Preset snapshot and one aggregate digest over every observed
  state input;
- ordered semantic steps containing canonical target, optional logical resource, action, and safe
  diagnostic codes;
- the required permission set, missing declarations, and stable uncomputable-permission codes.

Step actions are limited to `none`, `create`, `update`, `remove`, `execute`, `preserve`, and
`blocked`. They describe review intent only. They do not contain destination bytes, argv, env
values, secret plaintext, or an executable action payload.

Permissions are sorted and duplicate-free. Contract v1 supports:

- filesystem read, write, remove, and execute for a reviewable path;
- network access to one host or any host;
- command execution by program identity, excluding arguments;
- administrator authorization;
- environment variable names classified as plain or secret, excluding values;
- stable system capability and optional resource identifiers.

Required permissions missing from the declaration set are retained as blockers. A planner that
cannot compute a permission records a stable code. A Plan is ready only when no permission blocker
and no blocked step remains.

## Snapshot and approval semantics

Snapshot digests use SHA-256 with length-framed fields and sorted observation labels. Preset
digests include every effective logical path, effective embedded/external/overlay layer, and exact
bytes. They intentionally exclude physical checkout roots, so moving an unchanged repository does
not invalidate approval while changing its trust layer does.

State planners hash every observation that can affect steps or permissions, including runtime
manifests, receipts, live resource state, platform/context decisions, request mode, and opaque
secret handles or versions. Plain environment values contribute only a hash; decrypted secret
plaintext never enters a serializable Plan. Observation labels contain canonical targets and
logical resources rather than private source paths.

The App, Shell, and managed Sys planners are implemented only over filesystem and split-DNS
observation traits. Mutation host traits inherit those ports, but planner bounds cannot write,
remove, launch a process, request privilege, or apply system state. Target selection occurs before
host-state observation. Generators and hooks produce conservative `execute` and potential resource
steps without running code; existing external-code gates may still block them. Supported receipts
allow safe removal when the original Preset has disappeared, without reconstructing missing
teardown code.

`PlanApprovalV1` can be created only for a ready Plan. It stores the exact Plan fingerprint and
required permission set. Validation rejects unsupported schemas, blocked Plans, changed
permissions, or any changed fingerprint. A future apply path must re-plan from fresh captured
inputs and validate that result before the first mutation; approval is never a reusable grant.

## Delivery sequence

1. **Complete — contract foundation:** Plan/permission/snapshot/approval types, deterministic
   Preset digest, safe serialization, and fail-closed tests.
2. **Complete — permission declarations:** versioned target-local Preset syntax, static validation,
   built-in migration, and explicit compatibility for existing coarse grants.
3. **Complete — pure planners:** App, Shell, and managed Sys read-only assessment that cannot
   invoke process, write, privileged, or external-code capabilities.
4. **Approval enforcement:** CLI rendering and confirmation followed by re-plan, exact approval
   validation, and only then existing Core execution.
5. **Trust migration:** move `allow_app_hooks` and `allow_sys_code` users to scoped declarations
   without silently expanding permissions; separately decide auto-generator status compatibility.

Each slice must preserve existing ownership, manifest, user-modification, external-code, and secret
handling invariants until its replacement is complete.

## Acceptance for the delivered slices

- Contract enum spellings and schema fields are serialization-tested.
- Permission sets sort and deduplicate; missing and uncomputable permissions block approval.
- Blocked Plans cannot produce approval.
- Any source/state digest, ordered step, or required-permission change invalidates approval.
- Preset digest changes for logical path, content, or effective trust layer, but not physical root.
- Serialized Plans and approvals contain no source/state content, env values, secret plaintext, raw
  command arguments, or physical source checkout paths.
- App, Shell, and managed Sys Plans compile and run with observation-only fake hosts; target,
  request mode, manifests/receipts, live ownership, and relevant input identities affect the state
  digest and fingerprint.
- Missing declarations, missing secret identity, foreign ownership, user modification, and opaque
  managed behavior fail closed or preserve state with stable diagnostic codes.
- The existing CLI, public manuals, and mutation behavior remain unchanged by the planner slice.
