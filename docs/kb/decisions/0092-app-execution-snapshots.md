# 0092 — App scripts execute from captured category copies

- **Status**: Accepted
- **Date**: 2026-09-26
- **Evidence**: `core/src/runtime/{app_script,app,planner}.rs`
- **Extends**: ADR 0038, ADR 0046, ADR 0091

## Context

App trust and approval bind immutable category bytes, but the executor reopened external script
paths. An edit after capture could therefore change the executed entrypoint or a relative helper
without changing the already-approved Plan. Copying only the entrypoint leaves helper imports and
the source/overlay environment contract exposed to the same problem.

## Decision

Every generator, script hook, artifact and teardown invocation receives a fresh category copy under
`shine_dir/runtime/app/<category>/<invocation>/source`. Only effective captured bytes enter it;
shadowed base bytes do not become an additional executable source. Effective overlay files also
receive a separate captured subset. The invocation directory is private on Unix, the native
entrypoint is executable, and command hooks continue to invoke their declared external command.

`SHINE_APP_DIR` and `SHINE_APP_SOURCE_DIR` point to the effective copy. `SHINE_APP_OVERLAY_DIR`
points to its captured overlay subset when an overlay is active. These paths are temporary;
persistent output uses `SHINE_STATE_DIR`, `SHINE_CACHE_DIR`, or `SHINE_APP_HTTP_DIR`.
The source-layer check for Bun package/lock declarations remains unchanged.

Pure Plans declare creation and removal under the category runtime root and observe that root.
Random invocation identifiers are execution details, never fingerprint or report input. Execution
cleans only its invocation directory after process success or error; a preparation failure also
attempts cleanup. Unrelated and concurrent invocation directories remain untouched.

## Consequences

- Modifying the original entrypoint, helper or overlay after capture cannot change this invocation.
- Scripts that previously wrote into their checkout through the App path variables must use the
  persistent output contract instead. Relative category imports retain their directory structure.
- Code is still unisolated: arbitrary host access, external commands, dependency downloads, or
  explicitly opening other paths are not constrained by snapshot delivery.
- Abrupt process termination can leave an invocation directory; it is never reused as executable
  input by a later operation. This temporary copy is not a managed-resource recovery journal.
