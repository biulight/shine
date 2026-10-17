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
    let runtime = discover_runtime_paths_read_only().context("resolving active Preset paths")?;
    if let Some(path) = path {
        let canonical = tokio::fs::canonicalize(path)
            .await
            .with_context(|| format!("resolving Preset path {}", path.display()))?;
        let (root, selected) = explicit_scope(&canonical)?;
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(root.clone()),
                overlay_root: None,
            },
        )
        .await?;
        let managed = runtime
            .managed_overlay
            .then_some(runtime.presets_overlay_dir)
            .flatten()
            .filter(|overlay| canonical.starts_with(overlay));
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

    let source = if runtime.is_external_presets {
        PresetSnapshotSource::External(runtime.presets_dir.clone())
    } else {
        PresetSnapshotSource::Embedded(core_runtime::embedded_preset_files())
    };
    let snapshot = capture_preset_snapshot(
        &RealHost,
        PresetSnapshotRequest {
            source,
            overlay_root: runtime.presets_overlay_dir.clone(),
        },
    )
    .await?;
    let managed = runtime
        .managed_overlay
        .then_some(runtime.presets_overlay_dir)
        .flatten();
    Ok(MigrationInputs {
        snapshot,
        scope: "active".to_string(),
        selected: None,
        shine_dir: runtime.shine_dir,
        managed_overlay: managed,
    })
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
