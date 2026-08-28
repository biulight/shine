# 0033 — AI preset authoring is skill-first with Shine CLI as the authority

- **Status**: accepted
- **Evidence**: `cli/src/preset_validation.rs`, `cli/src/apps/metadata.rs`,
  `cli/src/shells/metadata.rs`, `cli/src/sys/manifest.rs`, `skills/shine-preset-author/`

## Context

AI can lower the cost of authoring custom presets, but copying the full preset schema into every
agent integration would create version drift. Writing Codex, Claude, Cursor, or other client
configuration from Shine would also create a growing compatibility and uninstall surface unrelated
to preset lifecycle ownership.

RTK demonstrates the useful part of a thin integration: one Rust binary owns authoritative behavior
while agent-facing instructions remain small. Its client-specific hooks, plugins, and rule files
also show the maintenance cost of broad client mutation. Agent Skills provide a portable workflow
and knowledge package, while MCP is better suited to connecting a model to tools or remote context.
Local coding agents already have a shell and can call Shine directly.

Preset authoring additionally needs a safe machine-readable check. Existing runtime loaders are
host-selective and normally sit behind `Config::load_or_init()`, which can initialize state. Calling
normal install commands alone therefore cannot provide all-platform, no-execution validation.

## Decision

Shine ships `skills/shine-preset-author/` as a standard Agent Skill and includes it in the crate
package. Users register the directory through their client's native skill mechanism. Shine does not
detect or write client configuration.

The skill remains a thin workflow adapter. It verifies that the installed binary exposes
`shine preset validate`, scaffolds with that binary's `preset new` or `preset copy`, loads one
kind-specific reference, requires JSON validation, and performs only isolated dry-runs. It never
activates a source or overlay and never executes preset code.

`shine preset validate [PATH] [--format text|json]` is the authority for static validation. Top-level
routing handles it before config loading and update checks. It reads the requested repository,
category, or manifest directly; checks every supported operating-system branch; reuses app, shell, sys,
transform, environment, and Bun policy parsing; verifies referenced files; and never starts a
generator, hook, artifact, bootstrap script, process, or network request. Compatible app/shell
categories without metadata remain valid with a `legacy_metadata` warning.

The JSON report is versioned as `schema_version: 1`. Public report types live in
`preset_validation.rs`; diagnostics use stable severity and code fields. Validation errors return
exit status 1 without adding non-JSON output. `shell install --dry-run` resolves the runtime link
plan without extracting, rendering, linking, recording, or editing profiles so the skill can test
all three preset kinds in an isolated temporary config.

MCP is deferred. If clients without a local shell later need to author local Shine presets, a future
server may expose constrained scaffold, validate, examples, and install-plan tools by reusing the
versioned report and internal validators. It must not expose arbitrary command execution or automatic
activation.

## Consequences

- The installed Shine version, not a copied skill schema, decides what is valid.
- One skill can work across clients that implement the open directory convention.
- Static validation is safe to run against untrusted preset source and in CI.
- A repository report can contain valid and invalid categories while preserving deterministic
  ordering and aggregate counts.
- New preset schema rules must update the shared runtime/static validation path and retain stable
  diagnostic behavior or introduce a new report schema version.
- Remote model access and client registration remain outside Shine v1's ownership boundary.
