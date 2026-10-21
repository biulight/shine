//! Portable path checks independent of the compiling host.

use super::CategoryPath;
use crate::runtime::PresetSnapshot;

pub(super) fn validate_all_app_destination_branches(
    snapshot: &PresetSnapshot,
    category: &CategoryPath,
) -> anyhow::Result<()> {
    let logical = format!("app/{}/shine.toml", category.name);
    let Some(bytes) = snapshot.get(&logical) else {
        return Ok(());
    };
    let value: toml::Value = toml::from_slice(bytes)?;
    if let Some(destination) = value.get("dest") {
        validate_destination_value(destination)?;
    }
    if let Some(files) = value.get("files").and_then(toml::Value::as_array) {
        for file in files {
            if let Some(destination) = file.get("dest") {
                validate_destination_value(destination)?;
            }
        }
    }
    Ok(())
}

fn validate_destination_value(value: &toml::Value) -> anyhow::Result<()> {
    match value {
        toml::Value::String(path) => validate_destination_path(path),
        toml::Value::Table(table) if table.contains_key("base") => {
            let path = table
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("data-dir destination requires path"))?;
            if !is_portable_relative_path(path) {
                anyhow::bail!(
                    "data-dir destination path must be relative and stay inside its root"
                );
            }
            Ok(())
        }
        toml::Value::Table(table) => {
            for (platform, value) in table {
                let path = value.as_str().ok_or_else(|| {
                    anyhow::anyhow!("destination for {platform} must be a string")
                })?;
                validate_destination_path(path)
                    .map_err(|error| anyhow::anyhow!("invalid {platform} destination: {error}"))?;
            }
            Ok(())
        }
        _ => anyhow::bail!("destination must be a path string or platform table"),
    }
}

pub(super) fn validate_destination_path(path: &str) -> anyhow::Result<()> {
    if !is_portable_absolute_destination(path) {
        anyhow::bail!("destination root must be absolute after expansion");
    }
    if has_parent_segment(path) {
        anyhow::bail!("destination root must not contain '..'");
    }
    Ok(())
}

pub(super) fn is_portable_absolute_destination(path: &str) -> bool {
    path == "~"
        || path == "$HOME"
        || path.starts_with("~/")
        || path.starts_with("~\\")
        || path.starts_with("$HOME/")
        || path.starts_with('/')
        || is_windows_absolute(path)
}

fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    drive || path.starts_with("\\\\")
}

pub(super) fn is_portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !is_windows_drive_path(path)
        && !has_parent_segment(path)
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| part == "..")
}
