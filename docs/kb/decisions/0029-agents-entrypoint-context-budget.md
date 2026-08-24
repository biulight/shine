# 0029 — Keep `AGENTS.md` as a bounded instruction entry point

- **Status**: accepted
- **Date**: 2026-08-22
- **Evidence**: `AGENTS.md`, `docs/kb/README.md`, `docs/kb/architecture/module-map.md`,
  `docs/kb/preset-authoring.md`

## Context

The root `AGENTS.md` grew to 748 lines and 56,509 bytes by combining mandatory agent behavior,
commands, a per-file module tree, cross-module data flows, release rules, and detailed preset
authoring. Codex project-instruction discovery has a 32 KiB default combined limit, so late rules
could be omitted under the default configuration. Even with a raised limit, always injecting
feature-specific architecture consumes context and gives durable safety rules less prominence.

The repository already has a routed knowledge base for invariants, flows, decisions, operations,
and lessons, but `AGENTS.md` duplicated much of that material.

## Decision

Treat the root `AGENTS.md` as the repository instruction entry point:

- keep mandatory workflow, hard repository rules, high-frequency commands, common verification
  boundaries, and a compact ownership overview in the root file;
- keep it comfortably below the default instruction limit rather than relying on a local
  `project_doc_max_bytes` increase;
- put detailed module ownership and command routing in `architecture/module-map.md`;
- put shell/app/sys preset construction rules in `preset-authoring.md`;
- keep behavioral narratives, invariants, rationale, and operations in their existing KB owners;
- link to authoritative detail and state when agents must read it instead of copying that detail
  back into the root file.

Nested `AGENTS.md` files are reserved for real directory-scoped overrides. They are not the default
storage mechanism for reference material because Codex discovers them only along the startup
working-directory path.

## Consequences

- Repository-wide hard rules load early and remain below the default context ceiling.
- Agents read detailed knowledge only when a task enters the relevant area.
- Module moves update `architecture/module-map.md`, not the root entry point.
- New root content must describe repository-wide agent behavior or a genuinely high-frequency
  command/verification boundary. Feature explanations belong in the routed KB.
- Raising `project_doc_max_bytes` may be useful for exceptional local setups, but it is not a
  substitute for maintaining these ownership boundaries.

Official Codex discovery behavior and the default limit are documented in
[Custom instructions with AGENTS.md](https://developers.openai.com/codex/guides/agents-md).
