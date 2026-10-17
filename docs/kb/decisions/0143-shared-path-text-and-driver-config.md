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

## Validation

An isolated harness compares the nine original path functions extracted from `0d28f38` with the
shared helpers: 1134 comparisons match, including empty, relative, rooted, Unicode, NUL, mixed
separator and verbatim/UNC strings. Both original Sys config readers have identical bodies;
the retained implementation changes visibility only. Permanent regressions cover component
spelling (including non-UTF-8 Unix paths), private-home detection boundaries, Windows prefix
handling and driver config errors/value retention. Existing consumer suites cover metadata,
packing, fixtures, launchers/profiles, approved lifecycle execution and recovery.

Local macOS validation passed all 649 Core and 1066 CLI tests (two CLI tests skipped), plus
workspace Clippy with warnings denied. After the parameter-only warning cleanup, all 31 launcher
tests passed again and Windows Core checking, including tests, passed without warnings. Native
Windows/Linux execution was not performed. Formatting, spelling and KB relative links also pass.

A second review compared the extracted helpers directly with Git baseline `0d28f38` and reran
all 1134 differential comparisons without differences. Removing only the moved definitions and
new imports leaves the nine path consumers and managed Sys consumer identical to the baseline.
The review also verified 107 approval, path-identity, lock, receipt and recovery source files
unchanged. Existing contention, stale-approval, receipt-failure and explicit-recovery regressions
remain in the full pre-commit suite; no production correction was needed.
