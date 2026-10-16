# 0122 — Reuse release archives for cargo-binstall

- **Date**: 2026-10-17
- **Status**: accepted
- **Evidence**: root `Cargo.toml`, `.github/workflows/{package-assets,release}.yml`

## Context

The published crate is `shine-cli`, its binary is `shine`, and release assets use abbreviated OS and
architecture names. These names differ from cargo-binstall's defaults. Existing installers and
self-upgrade already consume these archives.

## Decision

- Keep existing archive names and their root-level `shine`/`shine.exe` layout.
- Publish `package.metadata.binstall` in the root crate, with `tgz`, `{ bin }{ binary-ext }`, and
  six exact target URL overrides. Only GNU Linux, Apple Darwin, and MSVC Windows targets are mapped.
- Preserve cargo-binstall's normal user fallback behavior. Release verification explicitly selects
  `crate-meta-data`, excluding third-party binaries and source compilation.
- After both crate and GitHub Release publication, verify the exact tagged crate version using
  registry metadata. Check all six downloads, then install and verify the version on the three
  native CI platforms. Use temporary installation and Shine configuration roots.
- Require verification before the stable `release` → `main` sync PR. RCs get the same binary checks;
  the moving `preview` channel remains separate.

## Consequences

No Rust runtime or archive format changes are needed. Metadata takes effect for newly published
crate versions; local-manifest checks can validate the mapping against an existing version's assets.
Download-only checks do not prove execution on the other architectures. Verification happens after
publication and cannot roll back a published crate or Release.
