# 0089 — One stable documentation version per major

- **Status**: accepted
- **Scope**: public documentation versions and release verification

## Decision

Publish only the latest stable documentation within each major, plus the current `Next` manual.
The retained stable versions are currently 2.2 and 1.8. A new minor release replaces the previous
same-major snapshot; a patch release refreshes its major.minor snapshot without creating another
version entry. Stable content must describe the released product, while unreleased changes remain
in `Next`.

The newest stable version uses the root URL. Older majors use their major.minor version paths;
`Next` uses `/next`. Retired minor-version URLs are removed without redirects and return 404.
Historical content remains available through Git history and release tags.

Remove retired English and Chinese snapshots, sidebar files, translation metadata, and version
configuration together. `pnpm check:versions` checks that each major occurs once and that the
version list, configuration, and resource inventory agree; documentation CI runs this check and
its regression tests before building. Release preparation remains responsible for selecting the
latest released snapshot and verifying both locales.

## Consequences

The version selector stays concise and same-major manuals do not accumulate on each release.
Bookmarks to retired minor-version routes stop resolving; no compatibility redirect is promised.
Older major-version manuals remain available for users who have not migrated.
