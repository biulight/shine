---
title: Image tools
sidebar_position: 4
---

# Image tools

`image-tools` provides batch compression, resizing, and conversion for JPEG, PNG, and WebP while
keeping the original images. Compression does not guarantee a smaller file.

## Install and use

Every machine that runs these commands needs Bun 1.3.14 or newer in `PATH`; Shine does not bundle
Bun. No separate image library is needed.

Inspect and install the category:

```bash
shine info shell/image-tools
shine install shell/image-tools
```

If you only need compression, install `shell/image-tools/img-compress` instead. Open a new terminal
or reload your shell profile, then choose the operation you need:

| Operation | Example |
| --- | --- |
| Compress a file and the images in a directory | `img-compress photo.jpg screenshots/` |
| Resize to a maximum width of 1280 and save to another directory | `img-resize --width 1280 --output-dir ./resized photos/` |
| Convert to WebP at quality 75 | `img-convert --format webp --quality 75 --output-dir ./webp hero.png gallery/` |

For updates and removal, follow [Manage shell presets](../guides/shell-presets.md).

Each command accepts multiple file or directory inputs. A directory scan processes direct JPEG,
PNG, and WebP children only; it never recurses. Without `--output-dir`, output stays beside its
source as `photo.compressed.jpg`, `photo.resized.jpg`, or the selected conversion extension. An
output directory flattens the selected inputs, so duplicate target names fail explicitly. Existing
files also fail unless `--force` is present, and source images are never modified in place.
`--force` does not permit replacing an input image.

Batch processing continues after an individual failure and returns a nonzero status if any item
failed. The first 20 failures appear in the terminal. If there are more, the complete list is also
written to a uniquely named `image-tools-errors-*.log` under `--output-dir`, or the current directory
when no output directory was supplied.

`IMAGE_QUALITY`, `IMAGE_MAX_WIDTH`, and `IMAGE_MAX_HEIGHT` default to `80`, `1920`, and `1080`.
Quality settings apply to JPEG/WebP; PNG uses a fixed compression level. Use `--quality`, `--width`,
or `--height` for one invocation, or `shine env set` to keep different defaults on this machine; see [environment settings](../guides/environment.md#inspect-and-set-values).
In the default snapshot mode, changing a copied or external source still requires `shine upgrade`
before the installed command changes.
