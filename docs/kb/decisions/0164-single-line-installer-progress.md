# 0164 — Limited single-line installer progress

- **Status**: Accepted
- **Date**: 2026-11-01
- **Extends**: ADR 0100, ADR 0162, ADR 0163

## Context

Treating every carriage return as a completed log floods the terminal with progress snapshots.
Native terminal passthrough would lose per-row source labels and bounded diagnostics, and would
allow installers to overwrite Shine's history. Most download/build progress needs only one row.

## Decision

Keep the existing supervised pipes, approval and success checks. Core preserves semantic boundaries
using ProcessOutput codes: `sys_bootstrap_installer` completes a line, `_progress` represents bare
CR, `_partial` carries a bounded unfinished fragment, and `_tick` schedules presentation refresh.
A deferred CR distinguishes CRLF even across reads. Timed flushes still expose unterminated input
prompts; EOF commits fragments already sent. Raw diagnostic tails remain unchanged and bounded.

CLI's InstallerOutput owns per-stream style/pending buffers (64 KiB each) and one active row.
It coalesces progress-only redraws with an 80 ms minimum interval; 100 ms Core ticks ensure a final
pending update becomes visible even while the child is quiet. Complete logs print immediately,
clearing and restoring another stream's active row. Competing progress streams commit the previous
row. Outcomes, ordinary execution errors and observer drop finalize pending output before status.

Only Shine generates CR/clear-current-line commands. Child ANSI remains filtered under ADR 0163.
Live rows use ANSI-aware display width, preserve the source label and reserve the final terminal
column to avoid wrapping; subsequent redraws use the current width. Very narrow terminals,
TERM=dumb and redirected stdout use append-only tagged logs. NO_COLOR disables styles, not progress.
Full stable log lines can wrap normally. This is not a virtual screen: resizing may reflow existing
terminal history, and multi-line/cursor-up interfaces are unsupported.

A frontend-selected cosmetic preference supplies CARGO_TERM_PROGRESS_WHEN=always and an initial
CARGO_TERM_PROGRESS_WIDTH excluding the widest selected item tag. Ambient/request/configured
settings take precedence. Defaults are injected before sudo setup, only for installers, never
for detection or generators. No PTY, raw input mode, cursor hiding, new permission or CLI flag is
introduced. Existing timeout, cancellation and process-group cleanup remain independent.

## Consequences

Progress stays attributable without filling terminal history with every update. File logs keep
plain snapshots; unusually long unfinished rows flush in bounded fragments. Other installers may
suppress progress on pipes unless they expose their own setting. Terminal rendering is a local
capability rather than a second process executor or generic terminal emulator.
