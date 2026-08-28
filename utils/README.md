# shine-core

Shared, UI-agnostic helpers and lifecycle contracts for the Shine CLI and future Shine
applications.

The crate currently provides comment-preserving TOML table synchronization and
helpers for creating `shine.toml` preset metadata files, plus the versioned frontend-neutral
lifecycle result envelope used during the incremental Core extraction.
The lifecycle envelope covers App, Shell, and managed Sys adapters and distinguishes read-only
`pending` work from explicit dry-run `previewed` work.

The workspace-internal runtime module now provides captured runtime inputs, immutable Preset
snapshots, real/in-memory host ports, Core-owned App/Shell/Sys manifest models, shared resource
ownership primitives, and the first host-neutral App and managed-Sys executors. These runtime APIs
are intentionally hidden from normal documentation until the Phase 2 migration is complete.
