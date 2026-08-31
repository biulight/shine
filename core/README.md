# shine-core

Reusable, UI-agnostic lifecycle runtime and domain core for the Shine CLI and future Shine
applications.

The crate owns captured runtime inputs, immutable preset snapshots, real/in-memory host ports,
App/Shell/Sys lifecycle orchestration, manifest models, shared resource-ownership primitives,
preset validation, persistence helpers, and the versioned frontend-neutral lifecycle result
envelope. It also owns the Phase 3 contract foundation for semantic security Plans, deterministic
source/state snapshot digests, permission resolution, exact Plan approval, versioned target-local
Preset permission declarations, and snapshot-scoped external-code trust grants. Workspace-internal pure planners assess App, Shell,
managed Sys lifecycle, exact Sys bootstrap, App refresh/artifact, and explicit Sys profile requests
from immutable Presets plus observation-only filesystem and split-DNS ports.

The Roadmap Phase 4 foundation keeps executable `ActionIrV1` separate from the security Plan.
Approved App install routes static Copy files with absent destinations or
backup-eligible unowned regular-file destinations through the action executor. Approved install and
upgrade also route unchanged, receipt-owned in-place static Copy replacement through
same-directory transaction rollback material. Ordinary removal of an unchanged, receipt-owned,
static Copy uses the same transaction path until receipt absence and its positive
journal commit marker are durable; when the receipt owns a fixed persistent backup, the action
restores that user file through a second fingerprint-bound rename. Forced removal of a
user-modified file uses a distinct action at the same static Copy boundary, stages the
modified bytes as fingerprint-bound rollback material, and reverses an optional backup restoration
until receipt commit. Administrator create, update, and removal reuse these actions, derive explicit
Administrator permission, hold the privileged-operation lock through receipt commit, and route
protected writes, moves, removals, and mode restoration through the host privilege port. Each
journal remains until its matching manifest receipt state is durable; a
fresh `app-recovery` Plan is required before removing or restoring unchanged transaction state.
JSON merge, generators, relocation, and other domains retain their
existing executors until narrower rollback contracts land.

Runtime APIs are workspace-internal and hidden from normal documentation. The versioned lifecycle
result and security Plan contracts retain their documented compatibility guarantees. Protected
App, Shell, managed Sys, Sys bootstrap, App refresh/artifact, and Sys profile CLI mutations review
and freshly revalidate Plans; existing dry-run behavior remains a separate preview. Planning does
not write, execute Preset code, request privilege, or apply system state. Permission declarations,
durable external-code trust, and one-shot Plan approval remain separate contracts.
