# 0163 — Safe colors in tagged installer output

- **Status**: Accepted
- **Date**: 2026-11-01
- **Extends**: ADR 0038, ADR 0162

## Context

Cargo normally writes compilation progress to stderr and disables its automatic colors when
connected to a pipe. A stream suffix looks like severity, and escaping every ESC removes useful
styles. Raw terminal passthrough could erase source labels or alter the terminal independently of
Shine's results. Sequential execution does not make such controls necessary.

## Decision

Keep dim `[item]` labels for both child streams and plain `[shine]` status labels, all at column 5.
Labels help readers attribute copied or partial logs. Color handling is needed with or without
labels, so their small rendering cost does not justify removing that context.

CLI maintains a bounded parser and independent style state for each active item's stdout/stderr.
It permits only SGR reset, bold/dim, foreground/background colors (basic, indexed and RGB).
Each rendered body restores that stream's style and ends with a reset; no style reaches a label
or the following Shine result. OSC, DCS, cursor/erase commands, conceal and unsupported styles are
not forwarded. Incomplete sequences survive Core's timed/size fragments; CSI buffering is capped
at 128 bytes, control strings are discarded without retaining their payload. Item outcomes clear
parser state. Redirected or NO_COLOR output strips styles as well as terminal commands.
Literal tabs in text are rendered as four spaces, preserving indentation without forwarding
terminal tab controls. Literal backslash escape text is unchanged; other controls stay encoded.

The CLI enables an explicit Core runtime cosmetic preference only on a color-capable stdout TTY
without NO_COLOR or an ambient CARGO_TERM_COLOR override. Core defaults off and injects the
CARGO_TERM_COLOR=always fallback only into installer environments before sudo construction;
request/configured values win. It adds no permission, relaxes no approval check, changes no
success criterion and does not enable colors for probes, generators or other processes. Other
installers keep their own color-detection behavior. No generic FORCE_COLOR override or PTY is used.

## Consequences

Cargo progress regains colors through pipes while source labels and multi-line indentation stay
stable. [ADR 0164](0164-single-line-installer-progress.md) adds CLI-managed single-line progress;
arbitrary cursor commands and full terminal emulation remain outside this capability. The parser is intentionally narrower than a terminal, and unsupported styles are omitted.
Raw diagnostic tails remain bounded and separate from presentation. Color and plain-output
fixtures must cover fragmented sequences, independent streams, reset isolation, explicit settings,
NO_COLOR, redirection and rejected terminal commands.
