# ADR 0143: Share identical path text and Sys driver config helpers

- **Date**: 2026-10-18
- **Status**: Accepted

## Decision

Replace five copies of native-component preset path joining with one private `path_text` helper.
Bootstrap, App/Shell metadata, validation and pack keep their own source validation and traversal.
The helper preserves component spelling, lossy Unicode conversion and slash joining exactly.
It does not resolve paths or substitute for the root-aware `review_path` security encoder.

The same module owns the existing private-home-path predicate used by lint and synthetic fixtures,
and exact Windows verbatim-prefix removal used before launcher/profile quoting. Keep Unix case
sensitivity, the existing C-drive-only case-insensitive home check, UNC handling and all quoting
policies. These text operations do not authorize filesystem access.

Managed Sys execution reuses the identical manifest-owned non-empty driver config string reader.
Whitespace-only strings remain errors; accepted values are returned without trimming. Planner's
different string reader retains its separate empty-value policy and error text.

Seven redundant implementations are removed. No public API, source selection, Plan fingerprint,
approval gate, lock lifetime, receipt/journal schema or recovery order changes. Legacy migration
and receipt readers remain because production upgrade/removal still consumes them.
Name the launcher source-check Host parameter `_host` because only Unix uses it; the existing
Windows unused-parameter warning disappears without changing platform admission rules.

## Validation boundaries

Baseline comparison against `0d28f38` covers empty, relative, rooted, Unicode, NUL, mixed separator
and verbatim/UNC path strings. Both original Sys config readers have identical bodies; the
retained implementation changes visibility only. Removing moved definitions and imports leaves
the original consumers unchanged.

Permanent regressions cover component spelling (including non-UTF-8 Unix paths), private-home
detection boundaries, Windows prefix handling and driver config errors/value retention. Consumer
suites retain metadata, packing, fixtures, launcher/profile, approved lifecycle, contention,
stale-approval, receipt-failure and explicit-recovery coverage. Local macOS checks and Windows
Core cross-target compilation do not establish native Windows/Linux execution.
