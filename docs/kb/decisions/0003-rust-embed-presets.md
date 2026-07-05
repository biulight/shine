# 0003 — Presets are embedded in the binary via rust-embed

- **Status**: accepted
- **Evidence**: `cli/src/presets.rs` (`PresetAssets`), `cli/build.rs`

## Context

`shine` distributes shell scripts, app config presets, and OS bootstrap scripts. Shipping them as
separate files would require an installer, a download step, or a fixed filesystem layout.

## Decision

Everything under `presets/` is compiled into the binary with rust-embed and extracted to the
runtime presets dir on demand (`presets::extract_prefix`). `cli/build.rs` registers
`cargo:rerun-if-changed=../presets` so editing a preset triggers re-embedding.

External presets remain possible: `SHINE_PRESETS` / `presets_dir` point at a directory, and
`presets_overlay_dir` merges user files over embedded ones (commit `3f7ac41`). Embedded content
is always the fallback when an external file is missing (commit `5606438`).

## Consequences

- The binary is fully self-contained; `curl | sh` installation works with zero runtime deps.
- Adding a preset = adding files under `presets/` + rebuilding. No packaging step.
- Binary size grows with preset count (accepted trade-off).
