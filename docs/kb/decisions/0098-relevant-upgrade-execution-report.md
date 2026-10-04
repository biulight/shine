# 0098 — Show relevant results after upgrade approval

- **Status**: Accepted
- **Date**: 2026-10-04
- **Evidence**: `cli/src/{apps/upgrade.rs,shells/install.rs,sys/managed.rs,self_install.rs}`
- **Extends**: ADR 0095, ADR 0097

## Context

Upgrade review omitted unchanged App and Sys scopes, but the subsequent verbose execution report
still listed every current App file and managed Sys item. Shell also displayed the installed
category total next to one changed category. A no-op verbose run could end with an empty `Done`
summary.

## Decision

Ordinary upgrade execution reports show changed resources, preservation, conflicts, failures,
warnings, and meaningful runtime events. Verbose mode expands details for those results but omits
unchanged App rows, already-installed Sys rows, and installed Shell category totals. Managed Sys
progress labels for ordinary upgrade are omitted; relevant outcome lines still name their items,
so a current item does not create an empty section. `upgrade --verbose --full-plan` retains the prior complete
report alongside its complete Plan review.

The executor and structured lifecycle results still process every selected item. Only CLI events
are filtered. An ordinary fully unchanged run prints `Nothing to upgrade.` instead of an empty
`Done` line.
