# 0107 — Remove the local HTTP service and App publishing contract

- **Status**: Accepted
- **Date**: 2026-10-11
- **Supersedes**: [ADR 0035](0035-cross-platform-serve-user-service.md)
- **Evidence**: `cli/src/commands/cli.rs`, `core/src/runtime/{app,app_script,planner/app_code}.rs`

## Context

The built-in Surge preset installs profile includes and rule files beside the active profile.
Clash Verge installs local file-provider rules in mihomo's HomeDir. Neither requires Shine's
loopback server. No built-in script consumes the HTTP output variable. A read-only audit of the
locally configured external preset source, personal overlay, retained runtime presets, and installed
Surge/Clash Verge configurations found no references to the old service or publishing contract.
This audit establishes local migration readiness, not absence of every third-party consumer.

The local machine still had a legacy launchd registration and old published files. Their presence
is not evidence of a current preset dependency, and removing code must not silently delete them.

The server adds HTTP parsing, cache behavior, unauthenticated local file exposure, and three native
service-manager integrations without a required consumer in the supported preset workflows.

## Decision

- Remove the entire `shine serve` namespace, server implementation, and service installation code.
  Expose only removal of existing native registrations through `shine clear serve`.
- Remove `SHINE_APP_HTTP_DIR` from the fixed App environment shared by generators, hooks, artifacts,
  and teardown. Artifact Plans and execution no longer observe, grant, or create HTTP directories.
- Retain the state/cache output contract and snapshot-bound execution and approval behavior.
- Preserve legacy published files and service registrations during upgrades. `shine clear serve`
  explicitly stops/removes the fixed legacy user service, even after the old binary has been
  replaced or its configuration has become unreadable. It routes before config initialization and
  release checks, never removes published files, is repeatable, and reports service-manager errors.
- `clear` requires an explicit target and does not alias `state migrate`: the latter retains
  versioned runtime-state migration semantics from ADR 0020. New cleanup targets must declare
  their own scope rather than broadening the effect of existing targets.
- Retain the existing `guides/tasks-and-serve` document ID/URL for incoming links, while changing
  its title/content to tasks and application artifacts. Released documentation snapshots and
  release history remain historical; ADR 0035 is explicitly superseded.

## Consequences

This is an incompatible CLI and script-contract change and must be included in the next major
release under the repository's versioning convention. External consumers must migrate local
resources to application-supported files or arrange their own HTTP hosting when a URL is required.
No automatic cleanup or live application/service mutation occurs during ordinary upgrades.
