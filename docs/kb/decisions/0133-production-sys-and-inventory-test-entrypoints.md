# ADR 0133: Test production Sys, inventory and installation messages

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Move the virtual managed-file roundtrip to `plan_managed_sys` and
`run_managed_sys_approved`, with a fresh reviewed approval for each install and uninstall.
Check destination bytes, persisted managed-file receipt and hash, backup restoration, receipt
removal and journal cleanup both with and without an original user file. Delete the independent
`apply_managed_sys_file` / `remove_managed_sys_file` implementations and their request types only
after this production-path regression passes. Keep the shared receipt conversion used by lifecycle
removal and all persisted formats.

Replace two CLI-only row-filter tests with process tests that install through `shine app install`
and read through `shine list`. Cover category deduplication, a missing installed file, exclusion of
an uninstalled category, and a partially installed category with a user-modified file. Assert that
listing leaves receipts and destinations unchanged. Remove the parallel test-only row filter;
production continues through frontend inventory and its existing CLI projection.

The bootstrap Observer calls one production installation-line renderer. Its test checks both
administrator suffix variants, indentation and bullet, replacing the test-only wording function.
Keep terminal styling and stdout placement unchanged.

Remove six uncalled CLI convenience wrappers (`handle_apply`, the two managed upgrade wrappers,
profile enable/disable and `handle_unbuild`) and the two helpers reachable only from the bulk
upgrade wrapper. Retain dispatch-facing approved handlers, prepared aggregate upgrade and the
targeted upgrade result path. Restrict seven internally consumed Core methods to `crate::runtime`:
Shell profile removal, managed profile removal, Sys detection and preview, split-DNS assessment,
and the two approved App relocation executors. Their bodies and internal callers remain intact.

## Boundaries and validation

This is internal cleanup. Legacy Shell profile removal, compatibility receipts, snapshot-bound
approval, lifecycle locks, privilege checks, receipt-before-commit ordering and explicit recovery
remain on their production paths. No command, Plan, persisted schema or public manual behavior
changes. Migrate and run the replacement tests before removing old implementations, then run
workspace tests, approval/capability doctests and Rust lint checks. Native Windows/Linux and real
administrator/provider execution still require separate platform validation.
