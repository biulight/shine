---
name: shine-preset-author
description: Create, customize, debug, or validate Shine app, shell, and sys presets. Use when a user describes configuration files, helper commands, or system bootstrap behavior they want managed by Shine, or asks to adapt a built-in Shine preset. Guides safe scaffolding, static validation, hypothetical authoring plans, and isolated dry-runs without activating presets or changing the user's current Shine configuration.
---

# Shine Preset Author

Create reviewable Shine presets while treating the installed `shine` binary as
the authority for templates and validation.

## Safety boundary

- Never run `shine preset link`, `shine preset overlay link`, a real install,
  upgrade, bootstrap, artifact, hook, generator, or preset script.
- Never edit the user's active Shine configuration or copy work into its active
  preset directory.
- Static validation may read preset files but must not execute them.
- `shine preset plan` is safe only as a hypothetical authoring report. Never
  describe it as an approval or apply its output.
- Run installation planning only with a fresh temporary `SHINE_CONFIG_DIR` and
  a dry-run flag. Remove only that exact temporary directory afterward.

## Workflow

1. Run `shine preset validate --help`, `shine preset lint --help`,
   `shine preset plan --help`, and `shine preset test --help`. If any is
   unavailable, stop and explain that
   the installed Shine version must be upgraded; do not guess an older schema.
2. Confirm the requested outcome, target operating systems, destination paths,
   required environment values or secrets, and whether to start from a built-in
   category. Ask and report in the user's language.
3. Choose exactly one kind and read its reference:
   - App configuration files → [references/app.md](references/app.md)
   - Executable shell helper commands → [references/shell.md](references/shell.md)
   - OS bootstrap or managed system resources → [references/sys.md](references/sys.md)
4. Work in a conventional `<repository>/<kind>/<name>/` category directory.
   - For a new category, enter the empty category directory and run
     `shine preset new <kind>`.
   - To customize a built-in category, enter the repository or overlay root and
     run `shine preset copy <kind>/<built-in-name>`; the command creates the
     `<kind>/<built-in-name>/` path. Keep only files that really need
     customization so other behavior can continue following upstream.
5. Edit the generated or copied files. Keep every referenced source, script,
   fragment, package manifest, and lockfile inside the category. Use explicit
   `shine.toml` metadata even though legacy app and shell auto-discovery remains
   compatible. Keep every generated `schema_version = 1` permission declaration,
   classify environment names as `plain` or `secret`, and declare only identities —
   never arguments, values, ciphertext, credentials, or private checkout paths.
6. Run `shine preset validate <category-path> --format json`. Treat
   `valid: false` as blocking, fix diagnostics by their stable `code`, and rerun
   until `valid: true`. Warnings require a conscious explanation.
7. Run `shine preset lint <category-path> --format json`. Review every warning
   by stable code; do not use `--deny-warnings` merely to hide accepted findings.
8. For each requested target platform, run
   `shine preset plan <category-path> --platform <macos|linux|windows> --format json`.
   Review steps, permissions, opaque effects, and blockers. `ready: false` may
   reflect intentionally absent environment, trust, commands, or administrator
   state; explain it, but never invent those inputs or call the report approved.
9. If the category contains `shine.test.toml`, run
   `shine preset test <category-path> --format json` and fix every failed case.
   Never add executable fixture setup or teardown.
10. If the user requests a distributable bundle, write it outside the category
    with `shine preset pack <category-path> --output <file> --format json`.
    Never use `--force` to bypass a policy failure; it replaces only output.
11. Perform the isolated dry-run below. Do not substitute a real install.
12. Summarize the kind, generated files, supported platforms, validation, lint,
    and authoring-plan results, dry-run result or reason it was skipped,
    required environment values, and any remaining warning. Do not activate
    the category.

## Isolated dry-run

Create a fresh OS temporary directory, define its Shine config directory as a
child such as `<temp>/shine`, and copy only the completed category to
`<temp>/shine/presets/<kind>/<name>/`. Set `SHINE_CONFIG_DIR` for the single
command rather than exporting it globally.

- App: `SHINE_CONFIG_DIR=<temp>/shine shine app install <name> --dry-run`
- Shell: `SHINE_CONFIG_DIR=<temp>/shine shine shell install <name> --dry-run`
- Sys: run `SHINE_CONFIG_DIR=<temp>/shine shine sys bootstrap --dry-run` only
  when `<name>` matches Shine's current OS preset id. Otherwise report that the
  category received static all-platform validation only.

The temporary configuration may be initialized inside the temporary directory;
the user's existing Shine state must remain untouched. Do not supply secrets to
the dry-run. If a preset requires environment-backed generation, validate the
fallback source statically and report that generation was intentionally not run.
