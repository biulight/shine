# 0172 — Independent Sys initialization refresh

- **Status**: accepted
- **Date**: 2026-11-16

`sys init` uses a distinct `sys-init` Plan and approved entry under the Sys lifecycle lock.
It reads the complete run manifest but never selects, detects, installs, enables or records an
item. The existing composer, active/base/new/merge reconciliation and loader routing remain the
single implementation. Unsupported OS/shell pairs fail before publication.

External base delivery uses `sys/profile/<os>` with `sys-profile-code`, bound to the complete
`sys/<os>/` snapshot. It is derived only when a base file has external provenance; an empty
enabled set cannot authorize it. Enabled integrations retain their separate item requirements.
All applicable trust and composition checks finish before the first profile write. Human one-time
consent and persistent grants use the existing machinery; automatic confirmation grants neither.
The target is a security identity, not a new software item or activation record.

The Plan observes the complete manifest, effective preset, target shell, both startup files on
Linux, both phases' active/base/new/merge inputs and force-only backup permission. Generated files
and loading blocks retain bootstrap's non-transactional delivery. A conflict may coexist with
other completed writes; retry requires fresh review. No journal, receipt or recovery schema is
added, and ordinary managed recovery requirements remain unchanged. Existing binaries reject the
unknown serialized Plan operation rather than execute it; trust store schema and grant semantics
remain unchanged. Item grants never migrate into the new base identity.

Sys code review resolves item requirements using the request's OS, and CLI trust enrollment uses
its detected OS. The compatibility platform-only query cannot distinguish Arch from Ubuntu.
Explicit `sys/profile/<os>` grants are inspected using their own OS, including in trust listing.
A pending journal Plan has only a recovery blocker, with no fictitious profile-code identity.
