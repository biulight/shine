---
name: shine-preset-author
description: Create, customize, debug, or validate Shine app, shell, and sys presets, including adaptations of built-in categories. Use for preset source authoring and review; activation and live installation are outside this workflow.
---

# Shine Preset Author

Produce reviewable preset sources using the installed `shine` binary's templates and validator.
Infer routine details from the request and existing files. Ask only for missing choices that
materially affect the result; do not require a confirmation round before authorized authoring.
Follow the user's language for questions and results.

## Authorization boundary

- Author only in the requested workspace, outside active Shine configuration and preset roots.
  Never link a source/overlay, activate a preset, grant trust, or run a real install, upgrade,
  bootstrap, artifact, hook, generator, or preset script in this workflow.
- Validation is static. `shine preset plan` is a hypothetical report, never an approval or input
  to apply. Permission declarations describe requirements; they do not grant trust or authority.
- Use a fresh temporary `SHINE_CONFIG_DIR` for **scaffolding as well as runtime dry-runs**:
  `preset new` and `preset copy` can initialize config. Set it per command; clean up only the
  exact temporary directory created for this task. Never supply real secrets to checks.
- User instructions override workflow preferences; authoring authorization does not include
  activation. If a request also needs live changes, finish the reviewable sources first and
  identify the separate action and its authorization requirements. If this boundary blocks work,
  cite this file and the relevant rule rather than inventing an approval requirement.

## Authoring

1. Establish the outcome, target platforms, destinations, and required input **names** from the
   request or existing preset. Check `shine preset validate --help` once for the installed binary.
   Use `shine preset schema --format json` when report, fixture, or bundle contracts are needed;
   it describes those contracts and live help, not the entire preset TOML grammar. If a needed
   command is unavailable, report that limitation without guessing support or upgrading Shine.
2. Read only the reference for each kind being changed:
   [app](references/app.md), [shell](references/shell.md), or [sys](references/sys.md).
   A request spanning kinds may use more than one; unrelated kinds need no review.
3. Edit an existing category in place. For a new `<repository>/<kind>/<name>/`, run
   `SHINE_CONFIG_DIR=<temp>/shine shine preset new <kind>` from the empty category directory.
   For a built-in copy, run `SHINE_CONFIG_DIR=<temp>/shine shine preset copy <kind>/<name>`
   from the workspace root; it creates the category path. Preserve referenced support files
   and unrelated edits; do not scaffold over an existing category.
4. Keep explicit `shine.toml` metadata and referenced sources, scripts, fragments, and dependency
   files inside the category. Retain schema-v1 permission declarations at the proper target
   boundary; declare identities and environment sensitivity (`plain`/`secret`), never argv,
   values, ciphertext, credentials, or physical checkout paths in permission tables.

## Verify and deliver

- For changed preset sources, run `shine preset validate <category-path> --format json` and
  `shine preset lint <category-path> --format json`. Fix validation errors and review lint
  findings by stable code; explain accepted warnings. Validation errors block a ready deliverable.
  Rerun after fixes, not indefinitely when a missing tool or external input prevents progress.
- Run `shine preset plan <category-path> --platform <macos|linux|windows> --format json` for
  each requested target platform. Review steps, permissions, opaque effects, and blockers.
  Explain `ready: false` against its synthetic assumptions; never invent environment, trust,
  commands, or administrator state to make it ready.
- Use [verification details](references/verification.md) for existing `shine.test.toml` fixtures,
  requested bundles, and the isolated runtime dry-run. Rerun affected checks after changes;
  instruction-only edits need skill/link validation, not preset execution checks.
- Report files and supported platforms, checks and results, required input names, accepted
  warnings, and any skipped check or remaining blocker. Deliver sources without activation.
