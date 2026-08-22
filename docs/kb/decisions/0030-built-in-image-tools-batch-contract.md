# 0030 — Built-in image tools use safe, non-recursive batch outputs

- **Status**: accepted
- **Evidence**: `presets/shell/image-tools/`, `cli/src/config/mod.rs`

## Context

The public manual used a hypothetical Bun image workflow to explain how a shell preset can turn
shared scripts and machine-local values into installed commands. Making that example a built-in
preset requires a stable command contract for files, directories, output ownership, and partial
batch failures.

Recursive directory traversal would immediately require policies for hidden trees, generated
outputs nested below inputs, symlink boundaries, and preserved directory layouts. Overwriting
inputs by default would also conflict with Shine's broader safety posture, even though image output
files are user-owned rather than manifest-owned.

## Decision

Ship `shell/image-tools` as three cross-platform `runtime = "bun"` commands: `img-compress`,
`img-resize`, and `img-convert`. They accept multiple file or directory inputs, but directory inputs
scan direct regular JPEG, PNG, and WebP children only. A future recursive mode must be explicit.

Outputs are derived beside each source or flattened into an explicit `--output-dir`. Derived names
never equal the input name. Existing destinations fail unless `--force` is present, output
collisions are rejected, and encoded bytes are completed before a same-directory temporary file is
promoted to the destination.

Batch processing continues after per-file failures and returns nonzero when any occur. Up to twenty
failures are printed in full. Larger failure sets print the first twenty and write every diagnostic
to a uniquely named local log. Portable codecs only are exposed so the public contract is the same
on macOS, Linux, and Windows.

`IMAGE_QUALITY`, `IMAGE_MAX_WIDTH`, and `IMAGE_MAX_HEIGHT` are ordinary machine-local Shine values
with defaults of `80`, `1920`, and `1080`. Command-line options may override them for one run.

## Consequences

- The built-in example works immediately after installation when Bun 1.3.14 or newer is available.
- Batch jobs never silently recurse or modify their source images.
- Multiple input roots may collide when flattened into one output directory; those items fail
  explicitly instead of inventing a directory layout.
- Adding HEIC, AVIF, recursive traversal, or in-place editing requires a separate explicit contract.
