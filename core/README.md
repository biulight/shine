# shine-core

Reusable, UI-agnostic lifecycle runtime and domain core for the Shine CLI and future Shine
applications.

The crate owns captured runtime inputs, immutable preset snapshots, real/in-memory host ports,
App/Shell/Sys lifecycle orchestration, manifest models, shared resource-ownership primitives,
preset validation, persistence helpers, and the versioned frontend-neutral lifecycle result
envelope.

Runtime APIs are workspace-internal and hidden from normal documentation. The versioned lifecycle
result contract retains its documented compatibility guarantees.
