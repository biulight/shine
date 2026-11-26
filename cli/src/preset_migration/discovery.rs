//! Read-only source discovery and named migration inputs.
use super::*;

pub(super) struct MigrationInputs {
    pub(super) snapshot: PresetSnapshot,
    pub(super) scope: String,
    pub(super) selected: Option<BTreeSet<String>>,
    pub(super) shine_dir: PathBuf,
    pub(super) managed_overlay: Option<PathBuf>,
}

pub(super) async fn migration_inputs(path: Option<&Path>) -> Result<MigrationInputs> {
    let mut runtime = discover_migration_paths()
        .await
        .context("resolving active Preset paths")?;
    let managed = match runtime.managed_overlay_dir {
        Some(dir) => Some(canonical_source_root(dir).await?),
        None => None,
    };
    if let Some(path) = path {
        let canonical = tokio::fs::canonicalize(path).await.with_context(|| {
            format!(
                "resolving Preset path {}",
                crate::output::escape_controls(&path.display().to_string())
            )
        })?;
        let (root, selected) = explicit_scope(&canonical)?;
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(root),
                overlay_root: None,
            },
        )
        .await?;
        let scope = selected
            .as_ref()
            .and_then(|targets| targets.iter().next())
            .cloned()
            .unwrap_or_else(|| "explicit-repository".to_string());
        return Ok(MigrationInputs {
            snapshot,
            scope,
            selected,
            shine_dir: runtime.shine_dir,
            managed_overlay: managed,
        });
    }

    // Compare edit origins and the globally owned checkout in the same path namespace,
    // including when a project selects another overlay or refers to the mirror as its base.
    if managed.is_some() {
        if runtime.is_external_presets {
            runtime.presets_dir = canonical_source_root(runtime.presets_dir).await?;
        }
        if let Some(dir) = runtime.presets_overlay_dir {
            runtime.presets_overlay_dir = Some(canonical_source_root(dir).await?);
        }
    }
    let source = if runtime.is_external_presets {
        PresetSnapshotSource::External(runtime.presets_dir)
    } else {
        PresetSnapshotSource::Embedded(core_runtime::embedded_preset_files())
    };
    let snapshot = capture_preset_snapshot(
        &RealHost,
        PresetSnapshotRequest {
            source,
            overlay_root: runtime.presets_overlay_dir,
        },
    )
    .await?;
    Ok(MigrationInputs {
        snapshot,
        scope: "active".to_string(),
        selected: None,
        shine_dir: runtime.shine_dir,
        managed_overlay: managed,
    })
}

async fn canonical_source_root(path: PathBuf) -> Result<PathBuf> {
    match tokio::fs::canonicalize(&path).await {
        Ok(root) => Ok(root),
        // A configured checkout may not have been created yet.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error).with_context(|| {
            format!(
                "resolving Preset source {}",
                crate::output::escape_controls(&path.display().to_string())
            )
        }),
    }
}

pub(super) fn explicit_scope(path: &Path) -> Result<(PathBuf, Option<BTreeSet<String>>)> {
    let category = if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) != Some("shine.toml") {
            bail!("Preset migration file input must be shine.toml");
        }
        path.parent()
            .context("shine.toml has no category directory")?
    } else {
        path
    };
    if let (Some(name), Some(kind_dir)) = (
        category.file_name().and_then(|name| name.to_str()),
        category.parent(),
    ) && let Some(kind) = kind_dir.file_name().and_then(|name| name.to_str())
        && matches!(kind, "app" | "shell" | "sys")
    {
        let root = kind_dir
            .parent()
            .context("Preset category has no repository root")?;
        return Ok((
            root.to_path_buf(),
            Some(BTreeSet::from([format!("{kind}/{name}")])),
        ));
    }
    if path.is_file() {
        bail!("shine.toml must be under app/<name>, shell/<name>, or sys/<name>");
    }
    Ok((path.to_path_buf(), None))
}
