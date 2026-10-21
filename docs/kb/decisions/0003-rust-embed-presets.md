# 0003 — Presets are embedded in the binary via rust-embed

- **Status**: accepted
- **Evidence**: `cli/src/presets.rs` (`PresetAssets`), `cli/build.rs`

## Context

`shine` distributes shell scripts, app config presets, and OS bootstrap scripts. Shipping them as
separate files would require an installer, a download step, or a fixed filesystem layout.

## Decision

Everything under `presets/` is compiled into the binary with rust-embed. Core captures those bytes
for host-backed runtime materialization; CLI asset extraction uses
`extract_embedded_prefix` for pristine copy and `PresetAssetReader::extract_all` for export with
an explicitly selected overlay ([ADR 0119](0119-explicit-preset-asset-readers.md),
[ADR 0136](0136-production-asset-and-config-test-owners.md)). `cli/build.rs` registers
`cargo:rerun-if-changed=presets` so editing a preset triggers re-embedding.

External presets remain possible: `SHINE_PRESETS` / `presets_dir` point at a directory, and
`presets_overlay_dir` merges user files over embedded ones (commit `3f7ac41`). Embedded content
remains the fallback for missing overlay assets (commit `5606438`); a full-external presets root
is authoritative for runtime capture.

## Consequences

- The binary is fully self-contained; `curl | sh` installation works with zero runtime deps.
- Adding a preset = adding files under `presets/` + rebuilding. No packaging step.
- Binary size grows with preset count (accepted trade-off).
