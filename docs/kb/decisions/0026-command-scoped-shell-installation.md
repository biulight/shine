# 0026 — Shell commands are independently installable within shared categories

- **Status**: accepted
- **Evidence**: `cli/src/shells/{metadata,install,uninstall,deployment}.rs`,
  `cli/src/{shim,status,bin_links,completion}.rs`

## Context

Shell categories organize related preset sources, but their commands are often independent. The
`utils` category, for example, contains clipboard, environment-export, and theme commands. Requiring
the whole category to be activated made categorization dictate installation policy. It also made
`utils/shine-env-export` valid for inspection but a silent no-op when passed to scoped install.

Deployment cannot simply become file-scoped: external snapshot commands may depend on sibling files,
and embedded extraction is intentionally a category cache. At the same time, launchers and
`shell-manifest.toml` already identify individual commands.

## Decision

Accept `category/command` in scoped shell install/uninstall and
`shell/category/command` in top-level install/uninstall. Bare command aliases remain inspection-only;
mutation requires the category to avoid ambiguity.

Category sources and snapshots remain shared deployment material. Installation activates only the
selected command by creating its launcher, transform output, source wrapper, and manifest receipt.
Command-scoped manifest updates preserve sibling entries. Uninstall removes only the selected managed
entry and receipt, recalculates source wrappers, and cleans category material only after no installed
siblings remain. A foreign entry with the same command name is preserved.

Status uses a manifest receipt or legacy launcher as installation evidence. Source presence alone is
not installed state. Category install/uninstall retain their existing all-command behavior, and
category upgrade reconciles only commands already installed in that category; it never activates an
unselected sibling.

## Consequences

- Users can install `utils/shine-env-export` without also activating unrelated utilities.
- Preset authors can keep coherent source categories without creating one category per command.
- Shared snapshot changes can still affect every installed command that consumes that category, but
  unselected commands remain absent.
- Completion, help, status, tests, and both public manual locales expose the same target grammar.
