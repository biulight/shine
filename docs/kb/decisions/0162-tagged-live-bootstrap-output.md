# 0162 — Tagged live Sys bootstrap output

- **Status**: Accepted
- **Date**: 2026-11-01
- **Extends**: ADR 0038, ADR 0100, ADR 0161

## Context

Buffered installer logs leave long downloads and configuration steps looking stalled. Printing
raw child output directly would lose item attribution and let terminal controls overwrite status.

## Decision

Add an optional ProcessHost output channel. RealHost sends bounded byte chunks while using the
same process supervisor and bounded tail retention; non-streaming hosts deliver captured chunks
at completion. Core concurrently drains the channel into typed installer output events. It treats
CR/LF into typed presentation codes (extended by ADR 0164), caps line fragments at 8 KiB and flushes incomplete prompts every 100 ms
without splitting an incomplete UTF-8 character. No background task or unbounded queue is needed.

CLI uses `[item]` for both stdout and stderr and `[shine]` for its own start and result rows.
Stream identity remains in Core events; it is not a severity level. Cargo, for example, writes
normal compilation progress to stderr, so a distinct stderr label is misleading in ordinary use.
All source labels start at terminal column 5. Start arrows are bold cyan and
start descriptions are bold; source labels remain plain for Shine and dim for child output.
Result names remain bold with green success or red failure indicators. Child controls are filtered
as described in [ADR 0163](0163-safe-installer-colors.md), which permits
only validated color and intensity styles. Outcomes keep diagnostic tails but CLI does not
replay installer logs after completion; profile diagnostics, which are not streamed, still print.
Installation success remains exit status followed by declared detection. Approval, stdin, timeouts,
cancellation and process-group cleanup are unchanged.

## Consequences

Output becomes visible as the installer writes it; child buffering can still delay it. Cross-pipe
order reflects observation rather than a guaranteed original chronology. [ADR 0164](0164-single-line-installer-progress.md) renders CR-based progress in place on capable
terminals and keeps separate tagged lines in redirected output. Long or unterminated lines may be split into tagged fragments.
Homebrew-style phase headings inform presentation, but Shine does not parse provider-specific
logs into inferred phases or claim an installer message proves success.
