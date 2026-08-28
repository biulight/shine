# shine-core

Shared, UI-agnostic helpers and lifecycle contracts for the Shine CLI and future Shine
applications.

The crate currently provides comment-preserving TOML table synchronization and
helpers for creating `shine.toml` preset metadata files, plus the versioned frontend-neutral
lifecycle result envelope used during the incremental Core extraction.
The lifecycle envelope covers App, Shell, and managed Sys adapters and distinguishes read-only
`pending` work from explicit dry-run `previewed` work.
