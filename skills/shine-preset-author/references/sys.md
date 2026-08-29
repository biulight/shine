# Sys preset authoring

Use a sys preset for operating-system bootstrap items, shell integrations, or
managed system resources. The category name is Shine's OS id (for example
`macos`, `ubuntu`, or `windows`). Start from `shine preset new sys`; sys presets
must use `version = 2`.

## Bootstrap items

Every init item needs a unique `id`, non-empty `label`, one `detect` declaration,
and one `install` declaration. Detection may use a command, path, or non-empty
`any` probe list. Installation is either a supported package provider or a safe
relative script path. Keep installation scripts inside the category.

Profiles list known init item ids. `default_profile`, when present, must name a
defined profile. Profiles cannot include managed-mode items.

Every `[[items]]` entry has an item-local permission declaration with
`schema_version = 1`. Fixed package-provider mechanics and typed managed targets
remain structurally bounded; item scripts must conservatively declare their
executable Preset path plus any command, network, administrator, environment,
or system capabilities visible from source review. Do not infer that a
declaration makes opaque script behavior statically provable.

## Shell integration and managed resources

Each shell integration selects at least one supported shell and exactly one of
`path`, `env`, `eval`, `source`, `aliases`, or `fragment`. Fragment paths are
relative, remain inside the category, and must be valid UTF-8. Validation reads
but never executes fragments or scripts.

Managed items use `mode = "managed"` and a supported driver; they do not declare
bootstrap detect/install/shell fields. A `managed-file` driver requires an
in-category `config.source` and an absolute destination after expansion.

Permission declarations do not replace `allow_sys_code`; external or overlay
scripts and executable profile content remain blocked without that global
user opt-in.

Use static validation for every sys category. Run bootstrap dry-run only when
the category id matches the current host, and never run a real bootstrap from
this skill.
