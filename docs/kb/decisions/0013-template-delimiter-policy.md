# 0013 — One template delimiter (`@@VAR@@`) for every file type; per-type delimiters rejected

- **Status**: accepted
- **Evidence**: `cli/src/install_core/transforms/template.rs` (`@@[A-Za-z_][A-Za-z0-9_]*@@`),
  `cli/src/install_core/transforms/mod.rs` (bare-string transform registry),
  `cli/src/apps/metadata.rs` (`AppFile.transforms: Vec<String>`, `resolve_transforms`),
  `cli/src/shells/{metadata,template}.rs`, `presets/app/clash-verge/{shine.toml,merge.yaml}`,
  `docs/clash-verge-local-subscription-prd.md` §6.1 (line 149)

## Context

The `template` transform substitutes `[env]` values into preset files using the literal
`@@VAR@@` delimiter (plain string scan, not regex; name rule `@@[A-Za-z_][A-Za-z0-9_]*@@`). It is
currently used only on `.jsonc` (docker-engine/desktop), the extensionless ghostty theme configs,
and `.sh`/`.ps1` scripts — never on YAML.

`@` is a YAML reserved indicator character, which raised the question of whether the delimiter
should vary **by file type** so `@@VAR@@` could be used inside YAML. Two facts bound the answer:

1. **The YAML conflict is narrow.** `@` is illegal only as the **first character of a plain
   (unquoted) scalar**. Mid-scalar and quoted uses are legal:
   - `port: @@PORT@@` → invalid; `url: socks5://h:@@PORT@@` → valid; `key: "@@PORT@@"` → valid.
2. **Quoting works but loses type.** `key: "@@PORT@@"` → `key: "6152"` is a YAML *string*, not a
   number. So quoting is a sufficient escape hatch only when a string-typed value is acceptable;
   it does not satisfy a requirement for native YAML types (numbers/booleans).

Crucially, **no preset needs YAML templating today.** The one YAML preset,
`clash-verge/merge.yaml`, is installed as a plain `Copy` with no `transforms`, and its PRD
explicitly settled on "hardcode real values in the overlay, no templating, so the file is always
valid YAML" (`docs/clash-verge-local-subscription-prd.md:149`). Surge's `.conf` files follow the
same inert-example-plus-overlay pattern. The request that prompted this ADR was a *general design*
question, not a blocked task.

There is prior art for file-type-specific *syntax*: the `# shine-template: true` annotation
(`presets.rs::parse_template_annotation`) is `.sh`/`.ps1`-only because `#` is not a JS/TS comment.
So the design space was genuinely open.

## Decision

- **Keep a single delimiter, `@@VAR@@`, across every file type.** One placeholder syntax, one
  mental model for preset authors, one code path in `template.rs`.
- **Reject "delimiter auto-selected by file extension."** It is implicit magic (the same
  `transforms = ["template"]` line would silently mean different syntax per extension), requires
  threading a filename the transform pipeline (`transforms::apply(specs, bytes, env)`) does not
  carry, and is unreliable exactly where it matters (ghostty themes are extensionless; `.conf`
  could be Surge/INI/nginx). It also does not actually guarantee validity — even a YAML-specific
  `${}`/`{{}}` breaks inside a flow collection (`{a: ${X}}`).
- **YAML convention:** prefer the clash-verge pattern — hardcode real values in a presets overlay
  copy, install verbatim, no templating. When a string-typed env value in YAML is genuinely
  wanted, quote the placeholder (`key: "@@VAR@@"`); it is valid YAML before and after
  substitution.
- **If (and only if) env-rendered YAML with *native* types is ever required**, add an **explicit,
  opt-in, author-declared delimiter** — e.g. a `template_open`/`template_close` pair on the
  `[[files]]` entry, consumed only when `template` is in `transforms`, defaulting to `@@`/`@@` so
  all existing presets are unaffected — **not** extension inference. The native type comes from
  the substituted value's content (`port: ${PORT}` → `port: 6152`), so the delimiter only needs to
  be a valid unquoted YAML scalar before substitution and to vanish after it. This is deferred
  (YAGNI): transforms are currently bare strings with no parameter mechanism, and no preset needs
  it.

## Consequences

- Preset authors learn exactly one placeholder syntax; `template.rs`, `transforms/mod.rs`, and the
  `Vec<String>` transform model are unchanged.
- YAML presets that need env values stay on the clash-verge overlay pattern, or quote the
  placeholder when a string is acceptable. There is no supported way to template a *native-typed*
  value into YAML today, and that is intentional until a real need appears.
- Should that need appear, the path is pre-decided (explicit `template_open`/`template_close`),
  so the future change is additive and backward-compatible rather than a delimiter migration.
