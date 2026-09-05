# Preset verification details

Use this reference for fixture tests, requested bundles, or runtime dry-runs after static
validation/lint and hypothetical planning. These checks remain inside the skill's authoring boundary.

## Declarative fixtures

If the category contains `shine.test.toml`, run `shine preset test <category-path> --format json`
and fix failed cases. Add cases when changed behavior needs repeatable assertions; do not create
fixtures merely to restate a wording edit. Consult `shine preset schema --format json` for the
installed fixture contract.

Model only declared synthetic state: environment-name presence, opaque secret versions, files,
command detection, versioned receipt documents, exact trust selections, and administrator state.
Never add executable setup/teardown, real credentials, or private machine paths. Prefer exact
action, permission, and diagnostic sets when those expectations are intentionally stable.

## Requested bundles

Write a distributable bundle only when requested, outside the category:
`shine preset pack <category-path> --output <file> --format json`. Fix policy failures;
`--force` replaces the output file only and never bypasses policy.

## Isolated runtime dry-run

For changed presets, create a fresh OS temporary directory (or reuse this task's isolated
scaffolding directory) with a config child such as `<temp>/shine`. Copy the completed category
and its support files to `<temp>/shine/presets/<kind>/<name>/`. Set `SHINE_CONFIG_DIR` for each
command, never globally. Remove only that task-owned temporary directory afterward.

- App: `SHINE_CONFIG_DIR=<temp>/shine shine app install <name> --dry-run`
- Shell: `SHINE_CONFIG_DIR=<temp>/shine shine shell install <name> --dry-run`
- Sys: `SHINE_CONFIG_DIR=<temp>/shine shine sys bootstrap --dry-run`, only when `<name>`
  matches Shine's current OS preset id. Otherwise report static validation and hypothetical
  target-platform planning only; do not substitute a different host category.

Config initialization stays inside the temporary directory. Do not supply secrets, grant trust,
enable `--run-generators`, or substitute a real install. For environment-backed generation,
validate the fallback source statically and report that generation was not exercised. Missing
host prerequisites or unsupported dry-run commands are limitations to report, not reasons to
activate the preset or repeat an unchanged failing command.
