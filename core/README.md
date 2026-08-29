# shine-core

Reusable, UI-agnostic lifecycle runtime and domain core for the Shine CLI and future Shine
applications.

The crate owns captured runtime inputs, immutable preset snapshots, real/in-memory host ports,
App/Shell/Sys lifecycle orchestration, manifest models, shared resource-ownership primitives,
preset validation, persistence helpers, and the versioned frontend-neutral lifecycle result
envelope. It also owns the Phase 3 contract foundation for semantic security Plans, deterministic
source/state snapshot digests, permission resolution, and exact Plan approval.

Runtime APIs are workspace-internal and hidden from normal documentation. The versioned lifecycle
result and security Plan contracts retain their documented compatibility guarantees. The Plan
contract is not yet connected to CLI mutation paths; existing dry-run behavior is not a security
Plan.
