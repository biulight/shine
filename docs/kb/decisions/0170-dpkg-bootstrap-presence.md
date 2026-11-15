# 0170 — Observe dpkg package state before Docker bootstrap

- **Status**: Accepted
- **Date**: 2026-11-16

Ubuntu Docker bootstrap follows the official APT installation sequence, including removal of
conflicting distribution packages. Command presence cannot distinguish `docker.io` from Docker CE
or identify missing Compose/Buildx packages, so `dockerd` must not short-circuit this installer.

Add a declarative `dpkg` detection with required `packages` and optional `conflicts`. Planning and
execution share a parser for `/var/lib/dpkg/status`; planning performs no subprocess calls and
binds the database bytes to approval. Required packages must be in `ok installed` state. Conflict
packages in any state other than `not-installed` or `config-files` prevent skipping. Held but
otherwise fully installed packages count as present. Names describe package state, not verified
repository origin. Read/parse failures do not silently authorize an installer.

The installer uses `dpkg-query` after approval to select conflicts, removes them with APT, configures
the official signed source, installs the five official packages, starts Docker if needed and runs
`hello-world`. Failure stops the item; APT changes and containers are outside Shine rollback.
Review warns of possible conflict/dependent-package removal. No purge, autoremove, data deletion,
user-group changes, or automatic migration of arbitrary custom Engine installations is added.

Existing complete installations remain skipped, including a stopped service. Incomplete installations
enter the APT flow, which may update already-present packages while resolving the requested set.
Older Shine binaries do not understand `kind = "dpkg"`; test/deploy this preset with its matching binary.
