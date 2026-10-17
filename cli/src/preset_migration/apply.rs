//! Post-confirmation source rechecks, private backups and ordered atomic edits.
use super::*;

#[derive(Serialize)]
struct BackupManifest<'a> {
    schema_version: u32,
    files: Vec<BackupEntry<'a>>,
}

#[derive(Serialize)]
struct BackupEntry<'a> {
    logical_path: &'a str,
    source_layer: &'a str,
    original_sha256: String,
    mode: Option<u32>,
}

pub(super) fn migration_source_observations(
    snapshot: &PresetSnapshot,
    edits: &[PresetMigrationEdit],
) -> BTreeMap<PathBuf, Vec<u8>> {
    let targets = edits
        .iter()
        .map(|edit| {
            edit.logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<BTreeSet<_>>();
    snapshot
        .source_files()
        .filter(|(logical, _)| {
            let target = logical.split('/').take(2).collect::<Vec<_>>().join("/");
            targets.contains(&target)
        })
        .filter_map(|(_, file)| {
            file.origin
                .physical_path
                .as_ref()
                .map(|path| (path.clone(), file.bytes.clone()))
        })
        .collect()
}

pub(super) async fn ensure_sources_unchanged(sources: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    for (path, original) in sources {
        let current = tokio::fs::read(path)
            .await
            .with_context(|| format!("reading {} after review", path.display()))?;
        if current != *original {
            bail!("Preset source changed after review");
        }
    }
    Ok(())
}

pub(super) async fn create_backup_set(
    shine_dir: &Path,
    edits: &[PresetMigrationEdit],
    sources: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<PathBuf> {
    ensure_sources_unchanged(sources).await?;
    let root = shine_dir
        .join("preset-migration-backups")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&root).await?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).await?;
    let mut entries = Vec::new();
    for edit in edits {
        let backup = root.join(&edit.logical_path);
        persist::atomic_write_private(&backup, &edit.original).await?;
        entries.push(BackupEntry {
            logical_path: &edit.logical_path,
            source_layer: &edit.source_layer,
            original_sha256: sha256(&edit.original),
            mode: file_mode(&edit.physical_path).await?,
        });
    }
    let manifest = toml::to_string_pretty(&BackupManifest {
        schema_version: 1,
        files: entries,
    })?;
    persist::atomic_write_private(&root.join("manifest.toml"), manifest.as_bytes()).await?;
    Ok(root)
}

pub(super) async fn apply_edits(
    edits: &[PresetMigrationEdit],
    sources: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    ensure_sources_unchanged(sources).await?;
    for edit in edits {
        let current = tokio::fs::read(&edit.physical_path)
            .await
            .with_context(|| {
                format!("reading {} before migration", edit.physical_path.display())
            })?;
        if current != edit.original {
            bail!("Preset source changed after review: {}", edit.logical_path);
        }
        let permissions = tokio::fs::metadata(&edit.physical_path)
            .await?
            .permissions();
        match &edit.candidate {
            Some(candidate) => {
                persist::atomic_write(&edit.physical_path, candidate).await?;
                tokio::fs::set_permissions(&edit.physical_path, permissions).await?;
            }
            None => tokio::fs::remove_file(&edit.physical_path).await?,
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) async fn file_mode(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;
    Ok(Some(tokio::fs::metadata(path).await?.permissions().mode()))
}

#[cfg(not(unix))]
pub(super) async fn file_mode(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests;
