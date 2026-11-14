# 0169 — Ubuntu 22.04 userspace is the Linux release baseline

- **Date**: 2026-11-14
- **Status**: accepted
- **Evidence**: `.github/workflows/package-assets.yml`, `scripts/check-linux-glibc.py`

## Context

GNU binaries built directly on `ubuntu-latest` acquired GLIBC_2.38 and GLIBC_2.39
requirements and failed before startup on Ubuntu 22.04 (glibc 2.35). The Rust target triple
alone does not constrain native headers or the libc used at final link time.

## Decision

Build both existing GNU Linux targets inside `ubuntu:22.04`, keeping the hosted runner free
to evolve. Retain GNU targets and archive names so installers, self-upgrade and cargo-binstall
continue selecting the same assets. Separate caches from builds made on the newer userspace.

Before uploading each archive, reject glibc requirements above 2.35, including weak references,
then run version/help/schema checks against Ubuntu 22.04 libraries. Execute x86_64 natively and
aarch64 through QEMU with the matching cross sysroot. Both release channels share this gate.

## Consequences

Ubuntu 22.04 is the supported Linux binary baseline; older glibc and musl remain outside this
release contract. QEMU smoke checks do not replace native ARM lifecycle testing. Updating the
container baseline requires an explicit compatibility decision rather than a runner-image bump.
