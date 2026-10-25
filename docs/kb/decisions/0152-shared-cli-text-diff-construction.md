# 0152 — CLI text diffs share line construction, not business comparisons

- **Status**: Accepted
- **Date**: 2026-10-24
- **Evidence**: `cli/src/output.rs`, `cli/src/info/render.rs`,
  `cli/src/preset_migration/render.rs`, `cli/src/env/broker/{commands,policy}.rs`

## Decision

Use one private `output::unified_text_diff` function for the five CLI text-diff construction sites.
It splits only on LF, encodes optional file labels before constructing headers, emits complete
hunks with three context lines, and marks missing final LF itself. Standalone CR remains content,
including at EOF; `similar`'s broader newline interpretation must not join rendered change rows.
The function returns plain body text for the existing stdout encoder or SSH local-TTY encoder.

Keep comparison and presentation preparation with their owners. Info/update retain the UTF-8,
NUL and per-side 256 KiB checks, exact byte/status comparisons and installation data. Uniform CRLF
is normalized only on display copies with both styles identified. EOL-only changes remain visible
as updates; mixed LF/CRLF keeps individual terminators to avoid hiding their changed positions.
Migration keeps its preserved candidate bytes, mixed-input refusal and overlay preview rules.
Broker policies retain canonical TOML comparisons, complete approval text, default-No prompts,
nonce handling, lock scope and locked revalidation. Sys's content-merging algorithm is independent.

## Maintenance and verification

Moving migration's proven hunk writer into the existing output module removes independent
`from_lines` generation from Info and three broker sites without new modules, transport types or
policy switches. The added Info preparation is needed to distinguish real byte updates from
display noise; a common EOL-ignoring comparator would violate receipt and approval contracts.

Shared tests pin line structure, headers, EOF changes and complete multiple-hunk/long-line output.
Consumer tests retain migration behavior, byte-level App/Shell status and read-only inspection,
canonical policy serialization, and SSH rejection/replay/stale-review/long-approval boundaries.
