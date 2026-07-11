use crate::apps::{AppCategory, AppFile, load_active_categories, resolve_install_destination};
use crate::config::Config;
use crate::env::EnvConfig;
use crate::install_core::AppManifest;
use crate::shells::metadata::ShellCategory;
use crate::shells::metadata::load_active_categories as load_active_shells;
use crate::status::{FileStatus, build_shell_rows};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct AppShowFile {
    pub(super) category: AppCategory,
    pub(super) file: AppFile,
    pub(super) destination: PathBuf,
    pub(super) status: FileStatus,
    pub(super) manifest_entry: Option<crate::install_core::AppEntry>,
}

#[derive(Clone)]
pub(super) struct ShellShowFile {
    pub(super) category: ShellCategory,
    pub(super) file: crate::shells::metadata::ShellFile,
    pub(super) source_path: PathBuf,
    pub(super) rendered_path: PathBuf,
    pub(super) link_path: PathBuf,
    pub(super) link_target: Option<PathBuf>,
    pub(super) status: &'static str,
}

pub(super) async fn collect_app_files(config: &Config) -> Result<Vec<AppShowFile>> {
    let categories = load_active_categories(config, None).await?;
    let manifest = AppManifest::load(config.shine_dir()).await?;
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);

    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_key = format!("app/{}/{}", category.name, file.source_rel.display());
            let destination = match resolve_install_destination(&category, file, config) {
                Ok(dest) => dest,
                Err(e) => {
                    if manifest
                        .entries
                        .iter()
                        .any(|entry| entry.source == source_key)
                    {
                        eprintln!(
                            "warning: skipping {}/{}: {e:#}",
                            category.name,
                            file.source_rel.display()
                        );
                    }
                    continue;
                }
            };
            let Some(entry) = manifest.find_by_dest(&destination).cloned() else {
                continue;
            };
            let status =
                crate::status::app_entry_status(config, &category, file, &entry, env_map).await;
            files.push(AppShowFile {
                category: category.clone(),
                file: file.clone(),
                destination,
                status,
                manifest_entry: Some(entry),
            });
        }
    }

    Ok(files)
}

pub(super) async fn collect_shell_files(config: &Config) -> Result<Vec<ShellShowFile>> {
    let categories = load_active_shells(config, None).await?;

    let shell_rows = build_shell_rows(config).await?;
    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_path = config.preset_path(
                Path::new("shell")
                    .join(&category.name)
                    .join(&file.source_rel),
            );
            let rendered_path = config
                .rendered_dir()
                .join("shell")
                .join(&category.name)
                .join(&file.source_rel);
            let link_path = crate::bin_links::command_path_for_name(
                config.bin_dir(),
                std::ffi::OsStr::new(&file.command_name),
            );

            let source_exists = source_path.exists();
            let rendered_exists = rendered_path.exists();
            let (link_exists, link_target) = read_link_target(&link_path).await?;
            if !source_exists && !rendered_exists && !link_exists {
                continue;
            }

            let fallback_status = match (source_exists || rendered_exists, link_exists) {
                (true, true) => "up-to-date",
                (true, false) => "preset present, bin symlink missing",
                (false, true) => "bin symlink present, script missing",
                (false, false) => "not installed",
            };
            let label = format!("{}/{}", category.name, file.command_name);
            let status = shell_rows
                .iter()
                .find(|row| row.label == label)
                .map(|row| row.status_text)
                .unwrap_or(fallback_status);

            files.push(ShellShowFile {
                category: category.clone(),
                file: file.clone(),
                source_path,
                rendered_path,
                link_path,
                link_target,
                status,
            });
        }
    }
    Ok(files)
}

async fn read_link_target(path: &Path) -> Result<(bool, Option<PathBuf>)> {
    let is_link = tokio::fs::symlink_metadata(path)
        .await
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok((path.exists(), None));
    }
    let target = tokio::fs::read_link(path)
        .await
        .with_context(|| format!("reading symlink: {}", path.display()))?;
    Ok((true, Some(target)))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn renamed_docker_category_has_no_legacy_alias() {
        let dir = std::env::temp_dir().join(format!("shine-show-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let config = Config::new_for_test(&dir);
        tokio::fs::create_dir_all(config.shine_dir()).await.unwrap();

        let files = collect_app_files(&config).await.unwrap();
        assert!(files.iter().all(|file| file.category.name != "docker"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
