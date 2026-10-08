# 0101 — Preflight and roll back reported proxy installation failures

- **Status**: Accepted
- **Date**: 2026-10-09
- **Evidence**: `cli/src/env/proxy.rs`

## Context

Proxy installation wrote launchers and rules before reading its manifest. Invalid manifest state
therefore returned failure after enabling the proxy. Windows launcher sets also need one failure
boundary across all companions and persistent inputs.

## Decision

Prepare the complete file set and capture regular-file preimages before mutation. Parse rules and
manifest from those same bytes, retaining comment-preserving rule rendering. Existing launchers
must pass the ownership marker check. Recheck each file before writing; use private atomic temporary
files and apply the intended Unix mode. On a reported write error, restore touched files in reverse
order, including a replacement that became visible before synchronization failed. Restore previous
bytes and modes or remove a newly created file only if the current state still matches this
installation's expected content/mode states. Preserve concurrent edits and report rollback failure.

## Consequences

- Malformed or conflicting state fails before writing any installation file.
- Fault injection tests cover each launcher/rule/manifest write before and after visible replacement.
- Rollback state lives only in memory; process termination and noncooperating filesystem races are
  outside this guarantee. Empty parent directories may remain after failed installation.
- Proxy installation remains a CLI user-state operation; this adds no Preset trust or lifecycle
  approval authority and does not change uninstall semantics.
