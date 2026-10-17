//! Snapshot-only Sys metadata and referenced-file validation.

use super::{
    CategoryPath,
    destinations::{is_portable_absolute_destination, is_portable_relative_path},
    source::logical_path,
};
use crate::runtime::metadata_diagnostic::{MetadataDiagnosticError, MetadataDiagnosticKind};
use crate::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, SysDriverKind, SysInstall, parse_sys_manifest,
};
use std::path::Path;

pub(super) async fn validate_sys_category(
    runtime: &CoreRuntime<InMemoryHost>,
    category: &CategoryPath,
) -> anyhow::Result<bool> {
    let logical = format!("sys/{}/shine.toml", category.name);
    let bytes = runtime.presets().get(&logical).ok_or_else(|| {
        MetadataDiagnosticError::new(
            MetadataDiagnosticKind::MissingMetadata,
            format!("sys/{} requires a readable shine.toml", category.name),
        )
    })?;
    let text = std::str::from_utf8(bytes)?;
    let manifest = parse_sys_manifest(text)?;
    for item in &manifest.items {
        if let Some(SysInstall::Script { path, .. }) = &item.install {
            validate_snapshot_reference(runtime.presets(), &category.name, path, "install script")?;
        }
        for integration in &item.shell {
            if let Some(fragment) = &integration.fragment {
                let bytes = validate_snapshot_reference(
                    runtime.presets(),
                    &category.name,
                    fragment,
                    "profile fragment",
                )?;
                std::str::from_utf8(bytes).map_err(|error| {
                    anyhow::anyhow!("profile fragment must be valid UTF-8: {error}")
                })?;
            }
        }
        if item.driver == SysDriverKind::ManagedFile {
            let source = item
                .config
                .get("source")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("managed-file requires source"))?;
            validate_snapshot_reference(
                runtime.presets(),
                &category.name,
                source,
                "managed-file source",
            )?;
            if let Some(transforms) = item
                .config
                .get("transforms")
                .and_then(toml::Value::as_array)
            {
                let specs = transforms
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            anyhow::anyhow!("managed-file transforms must be strings")
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                crate::install::transforms::validate(&specs)?;
            }
            let target = item
                .config
                .get("target")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("managed-file requires target"))?;
            if !is_portable_absolute_destination(target) {
                anyhow::bail!("managed-file target must resolve to an absolute path");
            }
        }
    }
    Ok(true)
}

fn validate_snapshot_reference<'a>(
    snapshot: &'a PresetSnapshot,
    category: &str,
    relative: &str,
    label: &str,
) -> anyhow::Result<&'a [u8]> {
    if !is_portable_relative_path(relative) {
        anyhow::bail!("{label} must be a file inside the preset category");
    }
    let path = Path::new(relative);
    snapshot
        .get(&format!("sys/{category}/{}", logical_path(path)))
        .ok_or_else(|| {
            MetadataDiagnosticError::new(
                MetadataDiagnosticKind::MissingReference,
                format!("{label} is missing or unreadable"),
            )
            .into()
        })
}
