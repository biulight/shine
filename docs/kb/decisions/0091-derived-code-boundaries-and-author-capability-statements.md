# 0091 — Code boundaries are derived; author capabilities are statements

- **Status**: Accepted
- **Date**: 2026-09-16
- **Evidence**: `core/src/{plan,trust}.rs`, `core/src/runtime/{planner,trust,pack}.rs`,
  `cli/src/{lifecycle_plan,trust}.rs`
- **Supersedes in part**: ADR 0041, ADR 0046, ADR 0088, ADR 0090
- **Extends**: ADR 0064

## Context

Shine can derive its own structured file, launcher, environment-injection and elevation operations,
but it cannot prove the complete effects of arbitrary scripts or child processes. Requiring authors
to enumerate script commands, files, network access and system effects made incomplete declarations
look like enforcement. Empty declarations also became ceremony, while external Shell code could
avoid trust merely by omitting `opaque_code = "unrestricted"`.

External Shell snapshot deployment introduces a second issue: one category tree is shared by
several command receipts. Updating one selected command may replace code used by another installed
command, so selector-local trust checks are insufficient.

## Decision

Core separates three review domains:

1. derived Plan capabilities for operations Shine performs itself;
2. optional, unverified author capability statements;
3. automatically classified unisolated code boundaries and durable user trust.

Missing `commands`, `filesystem`, `network`, `system`, or an entire permission table does not block
planning or trust enrollment. Explicit values remain validated and are bound into the Plan, but are not trust identity. Environment declarations and Administrator authorization retain executor
semantics: Shine injects only declared inputs and requires explicit elevation approval.

App hooks, generators and artifacts; installed or sourced Shell commands; and Sys scripts or
executable profile integrations are classified from typed entry semantics. This classification does
not depend on `opaque_code`. The v2 unrestricted field and `preset new --unrestricted` remain
compatible authoring inputs, but cannot enable or disable the risk classification.

External code trust schema v2 binds each target and capability to its complete effective category
snapshot: `app/<category>/`, `shell/<category>/`, or `sys/<os>/`. The digest includes sorted logical
paths, bytes, and effective source layers. Snapshot trust requires matching content. Development
trust binds the same target, capability, source roots, and layers while
allowing content changes. Old v1 grants remain readable and revocable but never authorize new
execution without review. The earlier v2 shape was never released, so this decision replaces it in
place instead of introducing a synthetic v3 migration.

For external Shell code, snapshot trust permits snapshot deployment only. Live deployment requires
development trust. Existing live launchers are preserved; inspection marks targets without a valid
development grant as requiring review, and the next code-delivery mutation must satisfy the new
combination rule. Uninstall and recovery remain governed by ownership and recovery approval, without
requiring code trust.

When ADR 0064's category snapshot changes, planning expands the review set to every installed command
that depends on that shared tree. The Plan records selected and shared-resource-affected targets.
Every affected external target needs human operation consent or a matching grant before the first mutation; uninstalled
siblings are not enrolled or installed.

## Consequences

- Review text states that arbitrary code is unisolated and that author statements are not runtime
  file, network, or command restrictions.
- Static data copies remain structured operations even when a filename resembles a script.
- Empty permission tables and structurally duplicated command/path statements can be removed from
  built-ins and scaffolding while env/admin contracts remain explicit.
- A same-category auxiliary or data-file change deliberately invalidates snapshot grants.
- Plan/approval, authoring/review projections, and trust use schema v2; bundle and pack reports use
  v3. Historical journal and receipt formats are unchanged.
- This decision improves review accuracy and trust consistency. It adds no operating-system sandbox,
  script-body analysis, or per-invocation interception.

## Human operation consent and AI boundary

A trusted human-facing frontend may prepare a Plan with process-local snapshot grants for the
external targets shown by that Plan. It must obtain affirmative human consent before consuming the
review into an ApprovedOperation. The handoff carries exact snapshot grants, is neither cloneable
nor serializable, and is revalidated against fresh configuration, source and state before execution.
The review runtime clears temporary grants; nothing is saved in the trust store. Shell live remains
blocked without Development trust. Shared affected Shell targets participate in the same review.

Automatic `--yes` uses ordinary review and requires existing trust. Read-only/AI adapters receive
only reports, never the trusted frontend or approval constructor. This is a host capability boundary,
not a defense against arbitrary Rust callers or agents with unrestricted shell access. Such hosts
must separately restrict grant commands, direct script execution and synthetic terminal input.

Explicit info/update generator evaluation retains persistent trust; interactive app refresh provides
one-operation approval. Author statements (including env/admin) remain in the Plan and executor
contracts, not grant matching. Complete snapshot hashing still includes their metadata bytes.
Development trust means long-term authorization of future content for enrolled target/capability
and source roots/layers, not continuous code review. Neither trust mode constrains external tools,
dependencies, downloads or effects of running code.
