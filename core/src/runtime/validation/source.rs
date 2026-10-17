//! Observation-only discovery and immutable source capture.

use super::{CategoryPath, PresetDiagnostic, PresetSourceScope, error};
use crate::runtime::{FileKind, FileSystemObservationHost, PresetSnapshot, PresetSourceKind};
use std::path::{Path, PathBuf};

pub(in crate::runtime) async fn load_preset_source_scope(
    host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> Result<PresetSourceScope, PresetDiagnostic> {
    let display_path = absolute_path(cwd, path);
    let canonical = host
        .canonicalize(&display_path)
        .await
        .map_err(|error_value| {
            error(
                "invalid_input",
                format!(
                    "cannot resolve preset path {}: {:#}",
                    display_path.display(),
                    error_value.into_anyhow("canonicalizing preset path")
                ),
                &display_path,
            )
        })?;
    let categories = discover_categories(host, &canonical).await?;
    if categories.is_empty() {
        return Err(error(
            "invalid_input",
            "no preset categories found directly under app/, shell/, or sys/",
            &canonical,
        ));
    }
    let repository_root = common_repository_root(&categories);
    let snapshot = snapshot_tree(host, &repository_root).await?;
    Ok(PresetSourceScope {
        canonical,
        categories,
        repository_root,
        snapshot,
    })
}

async fn discover_categories(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Vec<CategoryPath>, PresetDiagnostic> {
    let metadata = host.metadata(path).await.map_err(|error_value| {
        error(
            "invalid_input",
            format!(
                "cannot inspect {}: {:#}",
                path.display(),
                error_value.into_anyhow("inspecting preset path")
            ),
            path,
        )
    })?;
    if metadata.kind == FileKind::File {
        if path.file_name().and_then(|name| name.to_str()) != Some("shine.toml") {
            return Err(error(
                "invalid_input",
                "preset manifest input must be named shine.toml",
                path,
            ));
        }
        return Ok(vec![category_from_root(
            path.parent().expect("canonical file parent"),
        )?]);
    }
    if metadata.kind != FileKind::Directory {
        return Err(error(
            "invalid_input",
            "preset path must be a directory or shine.toml",
            path,
        ));
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(is_kind)
    {
        return Ok(vec![category_from_root(path)?]);
    }
    let mut categories = Vec::new();
    for kind in ["app", "shell", "sys"] {
        let kind_root = path.join(kind);
        let Ok(metadata) = host.metadata(&kind_root).await else {
            continue;
        };
        if metadata.kind != FileKind::Directory {
            continue;
        }
        let entries = host.read_dir(&kind_root).await.map_err(|error_value| {
            error(
                "read_failed",
                format!(
                    "cannot read {}: {:#}",
                    kind_root.display(),
                    error_value.into_anyhow("reading preset category directory")
                ),
                &kind_root,
            )
        })?;
        for entry in entries {
            let entry_metadata = host.metadata(&entry).await.map_err(|error_value| {
                error(
                    "read_failed",
                    format!(
                        "{:#}",
                        error_value.into_anyhow("inspecting preset category")
                    ),
                    &entry,
                )
            })?;
            if entry_metadata.kind != FileKind::Directory {
                continue;
            }
            categories.push(CategoryPath {
                kind,
                name: entry
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                root: host.canonicalize(&entry).await.map_err(|error_value| {
                    error(
                        "read_failed",
                        format!(
                            "{:#}",
                            error_value.into_anyhow("canonicalizing preset category")
                        ),
                        &kind_root,
                    )
                })?,
            });
        }
    }
    categories.sort_by(|left, right| {
        (left.kind, left.name.as_str()).cmp(&(right.kind, right.name.as_str()))
    });
    Ok(categories)
}

fn category_from_root(root: &Path) -> Result<CategoryPath, PresetDiagnostic> {
    let kind = root
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|kind| is_kind(kind))
        .ok_or_else(|| {
            error(
                "invalid_input",
                "category directory must be app/<name>, shell/<name>, or sys/<name>",
                root,
            )
        })?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            error(
                "invalid_input",
                "preset category name must be valid UTF-8",
                root,
            )
        })?;
    Ok(CategoryPath {
        kind: match kind {
            "app" => "app",
            "shell" => "shell",
            "sys" => "sys",
            _ => unreachable!(),
        },
        name: name.to_string(),
        root: root.to_path_buf(),
    })
}

async fn snapshot_tree(
    host: &impl FileSystemObservationHost,
    root: &Path,
) -> Result<PresetSnapshot, PresetDiagnostic> {
    let mut builder =
        PresetSnapshot::builder(PresetSourceKind::External).base_root(root.to_path_buf());
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in host.read_dir(&directory).await.map_err(|error_value| {
            error(
                "read_failed",
                format!("{:#}", error_value.into_anyhow("reading preset snapshot")),
                &directory,
            )
        })? {
            let kind = host.metadata(&entry).await.map_err(|error_value| {
                error(
                    "read_failed",
                    format!(
                        "{:#}",
                        error_value.into_anyhow("inspecting preset snapshot entry")
                    ),
                    &entry,
                )
            })?;
            if kind.kind == FileKind::Directory {
                if entry.file_name().is_none_or(|name| name != "node_modules") {
                    pending.push(entry);
                }
            } else if kind.kind == FileKind::File {
                let relative = entry
                    .strip_prefix(root)
                    .map_err(|error_value| error("read_failed", error_value.to_string(), &entry))?
                    .to_path_buf();
                let bytes = host.read(&entry).await.map_err(|error_value| {
                    error(
                        "read_failed",
                        format!(
                            "{:#}",
                            error_value.into_anyhow("reading preset snapshot file")
                        ),
                        &entry,
                    )
                })?;
                builder = builder.file_with_executable(
                    logical_path(&relative),
                    bytes,
                    kind.unix_mode.unwrap_or(0) & 0o111 != 0,
                );
            }
        }
    }
    Ok(builder.build())
}

fn common_repository_root(categories: &[CategoryPath]) -> PathBuf {
    categories
        .first()
        .and_then(|category| category.root.parent())
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub(super) fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn absolute_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn is_kind(value: &str) -> bool {
    matches!(value, "app" | "shell" | "sys")
}
