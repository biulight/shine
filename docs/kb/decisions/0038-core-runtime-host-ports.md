# 0038 — Core runtime uses capability ports and immutable inputs

- **Status**: Accepted
- **Date**: 2026-08-29
- **Evidence**: `utils/src/runtime/`, `utils/src/runtime/sys_profile/`, `utils/src/install/`,
  `docs/shine-core-runtime-prd.md`

## Context

Phase 1 created a reusable lifecycle result, but lifecycle orchestration, configuration, preset
selection, manifests, and OS effects still live in the CLI library. Moving those functions as-is
would also move terminal, global environment, `rust-embed`, process, and administrator assumptions
into `shine-core`, preventing deterministic tests and reuse by a future UI.

Lifecycle progress must remain visible before later failures or prompts. At the same time, private
paths, raw errors, and frontend prose must not become fields of `LifecycleResultV1`.

## Decision

`shine-core` owns a workspace-internal `CoreRuntime`, domain request and assessment types, runtime
configuration, manifests, preset parsing, lifecycle orchestration, and OS-effect decisions. The
current package layout remains unchanged.

External capabilities enter Core through small host ports for filesystem, links, processes,
privileged mutations, and platform resources. Real and in-memory hosts implement the same ports.
Inputs such as home, cwd, environment, and preset contents are captured before execution rather
than read from ambient globals by domain logic.

Embedded assets remain distribution-specific. The CLI supplies them through an immutable preset
provider; Core owns source selection, overlay semantics, validation, and lifecycle decisions.

Frontend communication uses typed interaction requests and typed, non-serializable observer
events. The CLI owns dialoguer, styling, stream selection, and final wording. Observer data is a
side channel and must never be copied into the reusable lifecycle result.

The runtime API is public only because Rust crate boundaries require it and is hidden from normal
documentation. Phase 2 makes no third-party stability promise. `LifecycleResultV1` retains its
existing versioned compatibility promise.

Phase 2 assessment and existing dry-run behavior are not the Phase 3 Plan: they are not bound to a
source/state snapshot for approval and do not derive permissions.

## Consequences

- The dependency direction becomes mechanically enforceable as `shine-cli -> shine-core`.
- Complete lifecycle chains can run against an in-memory host without touching the real machine.
- CLI output remains compatible while Core stays independent of terminal libraries.
- Domain-specific models remain distinct; no generic action IR is introduced early.
- Moving a domain requires moving its manifests and resource decisions with it, not wrapping the
  old CLI executor behind a callback.
- App generator/hook/artifact/cache, Shell launcher/profile/live-render, Sys bootstrap/profile and
  split-DNS, preset validation, and App/Shell inspection therefore execute in Core. CLI adapters
  may render typed reports and events but may not retain a fallback resource executor.
