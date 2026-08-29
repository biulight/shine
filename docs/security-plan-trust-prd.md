# Shine Security Plan and Trust Model PRD

> **Status:** Roadmap Phase 3 contract foundation implemented; permission declarations, pure
> planners, CLI enforcement, and coarse-grant migration remain future slices. This document is
> internal and does not define released CLI, JSON, or Preset behavior.

## Summary

Roadmap Phases 1 and 2 established versioned lifecycle results, immutable Preset snapshots,
captured runtime context, Core-owned App/Shell/Sys execution, and real/in-memory host ports. Phase
3 adds a separate security boundary: every Preset lifecycle mutation must eventually be preceded by
a reviewable Plan that derives all required permissions and is bound to the exact source and state
inputs later applied.

The first slice adds the versioned Plan, permission, snapshot digest, fingerprint, and approval
contracts to `shine-core`. It does not route current commands through them. Existing dry-run and
status paths remain compatibility behavior and are not security Plans.

## Goals

1. Represent reviewable lifecycle intent without embedding executable actions or private payloads.
2. Cover filesystem, network, command, administrator, environment/secret, and system permissions.
3. Fail closed when a required permission is undeclared or cannot be computed.
4. Bind approval to exact Preset and observed-state snapshots, ordered semantic steps, and the
   complete required permission set.
5. Reject apply after any source, state, step, or permission change; permission expansion always
   requires a new review.
6. Keep planning free of host mutation, Preset code execution, and secret plaintext.

## Non-goals of the foundation slice

- No Preset permission syntax or migration of built-in Presets.
- No App, Shell, managed Sys, bootstrap, artifact, refresh, or profile planner.
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

Future state planners must hash every observation that can affect steps or permissions, including
runtime manifests, receipts, live resource state, platform/context decisions, and opaque secret
handles or versions. They must never hash decrypted secret plaintext into a serializable Plan.

`PlanApprovalV1` can be created only for a ready Plan. It stores the exact Plan fingerprint and
required permission set. Validation rejects unsupported schemas, blocked Plans, changed
permissions, or any changed fingerprint. A future apply path must re-plan from fresh captured
inputs and validate that result before the first mutation; approval is never a reusable grant.

## Delivery sequence

1. **Complete — contract foundation:** Plan/permission/snapshot/approval types, deterministic
   Preset digest, safe serialization, and fail-closed tests.
2. **Permission declarations:** versioned Preset syntax, static validation, built-in migration, and
   explicit compatibility for existing coarse grants.
3. **Pure planners:** App, Shell, and managed Sys read-only assessment that cannot invoke process,
   write, privileged, or external-code capabilities.
4. **Approval enforcement:** CLI rendering and confirmation followed by re-plan, exact approval
   validation, and only then existing Core execution.
5. **Trust migration:** move `allow_app_hooks` and `allow_sys_code` users to scoped declarations
   without silently expanding permissions; separately decide auto-generator status compatibility.

Each slice must preserve existing ownership, manifest, user-modification, external-code, and secret
handling invariants until its replacement is complete.

## Acceptance for the foundation slice

- Contract enum spellings and schema fields are serialization-tested.
- Permission sets sort and deduplicate; missing and uncomputable permissions block approval.
- Blocked Plans cannot produce approval.
- Any source/state digest, ordered step, or required-permission change invalidates approval.
- Preset digest changes for logical path, content, or effective trust layer, but not physical root.
- Serialized Plans and approvals contain no source/state content, env values, secret plaintext, raw
  command arguments, or physical source checkout paths.
- The existing CLI, Preset schemas, public manuals, and mutation behavior remain unchanged.
