# ADR 0147: Separate built-in preset contracts from engine and CLI regressions

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Use one root Cargo integration target, `builtin_presets`, with inventory, App, Shell, Sys and
capability-document modules. Keep it in the CLI package, which owns embedded distribution;
no new crate or public production API is needed. Local fixtures consume the public embedded reader
and Core snapshot/runtime interfaces. Test resources stay outside the embedded presets directory.
Keep generic engine, configuration assembly, CLI output, copy/export and lifecycle safety tests
with their production owners, even when those tests use built-in inputs.

Keep TypeScript tests beside their scripts, discovered by `bun test ./presets`. Extend the existing
Rust hook to preset and Cargo/toolchain inputs; retain workspace nextest and all three native CI
jobs. Include the integration target in Cargo source packages. Do not convert existing scenarios
into declarative fixtures as part of relocation.

## Coverage correspondence

| Previous owner/scenario | New owner and retained boundary |
| --- | --- |
| `preset_validation`: all built-ins valid | `inventory`: production validator, schema, zero errors/warnings, all three kinds |
| `presets`: proxy/utils discovery and descriptions | `shell`: embedded snapshot, Linux selection, category presence and every proxy description |
| `shells`: direct proxy script invocation | `shell`: Unix gate, both scripts, unsuccessful exit and source-required stderr |
| `apps`: Docker destinations | `app`: native Windows/Unix gates, exact destination through Core |
| `apps`: Clash per-file destination | `app`: both files, independent captured data directory, Core destination resolution |
| `core_runtime`: built-in bootstrap readiness | `sys`: macOS/Linux/Windows matrix, rust item, shell selection and ready Plan |
| `preset_meta`: capability docs and line endings | `docs`: production metadata, both locales, update opt-in, CRLF normalization/preservation and package skip |

The missing-category discovery assertion stays with the generic reader tests. Metadata adapter
smoke tests and Surge CLI rejection tests remain in CLI. No safety/approval assertions are removed.
Only fixture assembly and test entry points change; runtime behavior and resource filtering do not.

## Validation

Compare the scenarios and assertions above before and after migration, not just test counts.
Run the standalone target, workspace nextest, doctests, Bun checks and applicable quality checks.
Check Cargo package membership and automatic test discovery. Local platform simulation does not
replace native Windows/Linux/macOS CI or prove real administrator/provider execution.

Source-package paths are root-anchored and exclude nested `node_modules` explicitly. A separate
`source_package` integration target exercises Cargo's actual matching with required-source and
forbidden-tree fixtures, so packaging policy does not become a preset-content responsibility.
Preset fixtures use unique observation roots and retain host-specific profile selections; imports
used only by Unix assertions have the same platform gate as their consumers.
