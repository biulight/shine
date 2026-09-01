use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Write a `shine.toml` preset-metadata template into `dir`, refusing to
/// overwrite an existing file unless `force` is set.
///
/// Returns the written path and whether an existing file was overwritten.
pub fn write_shine_toml_template(
    dir: &Path,
    force: bool,
    template: &str,
) -> Result<(PathBuf, bool)> {
    let path = dir.join("shine.toml");
    let exists = path.exists();
    if exists && !force {
        bail!("shine.toml already exists; use --force to overwrite");
    }
    std::fs::write(&path, template).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, exists))
}
