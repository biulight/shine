//! Shared runtime bootstrap over explicit host capabilities.
//!
//! Frontends provide distribution-owned embedded bytes and resolved settings.
//! External directory discovery and snapshot construction stay reusable and
//! testable by observing the selected host through `FileSystemHost`.

use super::preset::PresetSnapshotBuilder;
use super::{FileKind, FileSystemHost, PresetSnapshot, PresetSourceKind};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum PresetSnapshotSource {
    Embedded(Vec<(String, Vec<u8>)>),
    External(PathBuf),
}

#[derive(Clone, Debug)]
pub struct PresetSnapshotRequest {
    pub source: PresetSnapshotSource,
    pub overlay_root: Option<PathBuf>,
}

pub fn capture_embedded_preset_snapshot(files: Vec<(String, Vec<u8>)>) -> PresetSnapshot {
    embedded_snapshot_builder(files).build()
}

fn embedded_snapshot_builder(files: Vec<(String, Vec<u8>)>) -> PresetSnapshotBuilder {
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for (logical, bytes) in files {
        builder = builder.file(logical, bytes);
    }
    builder
}

pub async fn capture_preset_snapshot(
    host: &impl FileSystemHost,
    request: PresetSnapshotRequest,
) -> Result<PresetSnapshot> {
    let mut builder = match request.source {
        PresetSnapshotSource::Embedded(files) => embedded_snapshot_builder(files),
        PresetSnapshotSource::External(root) => {
            let mut builder =
                PresetSnapshot::builder(PresetSourceKind::External).base_root(root.clone());
            for (logical, bytes) in capture_preset_tree(host, &root).await? {
                builder = builder.file(logical, bytes);
            }
            builder
        }
    };
    if let Some(root) = request.overlay_root {
        builder = builder.overlay_root(root.clone());
        for (logical, bytes) in capture_preset_tree(host, &root).await? {
            builder = builder.overlay_file(logical, bytes);
        }
    }
    Ok(builder.build())
}

async fn capture_preset_tree(
    host: &impl FileSystemHost,
    root: &Path,
) -> Result<Vec<(String, Vec<u8>)>> {
    match host.metadata(root).await {
        Ok(metadata) if metadata.kind == FileKind::Directory => {}
        Ok(_) => return Ok(Vec::new()),
        Err(error) if error.is_not_found() => return Ok(Vec::new()),
        Err(error) => return Err(error.into_anyhow("inspecting preset root")),
    }

    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = host
            .read_dir(&directory)
            .await
            .map_err(|error| error.into_anyhow("reading preset directory"))
            .with_context(|| format!("reading preset directory {}", directory.display()))?;
        entries.sort();
        for path in entries {
            let metadata = host
                .metadata(&path)
                .await
                .map_err(|error| error.into_anyhow("inspecting preset entry"))
                .with_context(|| format!("inspecting preset entry {}", path.display()))?;
            match metadata.kind {
                FileKind::Directory => {
                    if path.file_name().is_none_or(|name| name != "node_modules") {
                        pending.push(path);
                    }
                }
                FileKind::File => {
                    let relative = path
                        .strip_prefix(root)
                        .context("preset file escaped its snapshot root")?;
                    let bytes = host
                        .read(&path)
                        .await
                        .map_err(|error| error.into_anyhow("reading preset file"))
                        .with_context(|| format!("reading preset file {}", path.display()))?;
                    files.push((logical_path(relative), bytes));
                }
                FileKind::Symlink => {}
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::InMemoryHost;

    #[tokio::test]
    async fn external_and_overlay_discovery_use_the_selected_host() {
        let host = InMemoryHost::new();
        host.put_file("/virtual/base/app/demo/shine.toml", b"base".to_vec());
        host.put_file("/virtual/base/node_modules/ignored.js", b"ignored".to_vec());
        host.put_file("/virtual/overlay/app/demo/shine.toml", b"overlay".to_vec());

        let snapshot = capture_preset_snapshot(
            &host,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External("/virtual/base".into()),
                overlay_root: Some("/virtual/overlay".into()),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            snapshot.get("app/demo/shine.toml"),
            Some(b"overlay".as_slice())
        );
        assert!(snapshot.get("node_modules/ignored.js").is_none());
        assert_eq!(
            snapshot
                .origin("app/demo/shine.toml")
                .and_then(|origin| origin.physical_path.as_deref()),
            Some(Path::new("/virtual/overlay/app/demo/shine.toml"))
        );
    }
}
