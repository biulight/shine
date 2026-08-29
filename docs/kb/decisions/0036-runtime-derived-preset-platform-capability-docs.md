# 0036 — Built-in preset platform capability docs derive from runtime metadata

- **Status**: Accepted
- **Date**: 2026-08-27
- **Evidence**: `core/src/runtime/{app_metadata,shell}.rs`, `cli/src/preset_meta.rs`,
  `docs/manual/reference/built-in-presets.md`

## Context

Exact preset platform selectors prevent a macOS-only preset from being exposed on Linux or
Windows, but the public manual can still drift independently. A prose claim such as “macOS-only”
does not prove that the runtime destination and file filters omit the preset on the other two
operating systems.

## Decision

The built-in preset reference contains a delimited platform capability block for every App category
and Shell command. A host-independent Rust conformance test generates the expected rows from the
pristine embedded preset bundle using the same destination selection, file-level `platforms`, and
per-file destination rules as runtime loading. The test compares the complete generated block in
both the English and Simplified Chinese manuals. Setting `SHINE_UPDATE_PRESET_CAPABILITIES=1` for
that targeted test replaces both marked blocks; without it the test is read-only and reports drift.

System preset profiles remain in their existing platform table because sys routing selects an
OS-specific category rather than using App/Shell selectors.

## Consequences

- A selector change that exposes or hides a built-in capability fails tests until both manual
  locales are updated.
- The generated block is an availability index, not a claim that every corresponding third-party
  application is installed or fully supported by its vendor on that OS.
- External and overlay presets are excluded because the checked documentation describes the
  pristine bundle shipped by that Shine version.
