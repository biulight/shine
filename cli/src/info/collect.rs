use crate::apps::{AppCategory, AppFile, load_active_categories, resolve_install_destination};
use crate::config::Config;
use crate::env::EnvConfig;
use crate::install_core::AppManifest;
use crate::shells::metadata::ShellCategory;
use crate::shells::metadata::load_active_categories as load_active_shells;
use crate::status::{FileStatus, UpdateChange, assess_app_file, build_shell_rows};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct AppInfoFile {
    pub(super) category: AppCategory,
    pub(super) file: AppFile,
    pub(super) destination: PathBuf,
    pub(super) status: FileStatus,
    pub(super) manifest_entry: Option<crate::install_core::AppEntry>,
    pub(super) changes: Vec<UpdateChange>,
}

#[derive(Clone)]
pub(super) struct ShellInfoFile {
    pub(super) category: ShellCategory,
    pub(super) file: crate::shells::metadata::ShellFile,
    pub(super) source_path: PathBuf,
    pub(super) installed_source_path: PathBuf,
    pub(super) rendered_path: PathBuf,
    pub(super) link_path: PathBuf,
    pub(super) link_target: Option<PathBuf>,
    pub(super) status: &'static str,
    pub(super) changes: Vec<UpdateChange>,
}

pub(super) async fn collect_app_files(config: &Config) -> Result<Vec<AppInfoFile>> {
    let categories = load_active_categories(config, None).await?;
    let manifest = AppManifest::load(config.shine_dir()).await?;
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);

    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_key = format!("app/{}/{}", category.name, file.source_rel.display());
            if let Err(error) = resolve_install_destination(&category, file, config) {
                if manifest.find_by_source(&source_key).is_some() {
                    eprintln!(
                        "warning: skipping {}/{}: {error:#}",
                        category.name,
                        file.source_rel.display()
                    );
                }
                continue;
            }
            let assessment = assess_app_file(config, &category, file, &manifest, env_map).await;
            let Some(destination) = assessment.destination else {
                continue;
            };
            let status = assessment.status;
            if status == FileStatus::NotInstalled {
                continue;
            }
            let manifest_entry = manifest
                .find_by_dest(&destination)
                .or_else(|| manifest.find_by_source(&source_key))
                .cloned();
            files.push(AppInfoFile {
                category: category.clone(),
                file: file.clone(),
                destination,
                status,
                manifest_entry,
                changes: assessment.changes,
            });
        }
    }

    Ok(files)
}

pub(super) async fn collect_shell_files(config: &Config) -> Result<Vec<ShellInfoFile>> {
    let categories = load_active_shells(config, None).await?;

    let shell_rows = build_shell_rows(config).await?;
    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let label = format!("{}/{}", category.name, file.command_name);
            let Some(row) = shell_rows.iter().find(|row| row.label == label) else {
                continue;
            };
            if !row.is_installed {
                continue;
            }
            let source_path = config.preset_path(
                Path::new("shell")
                    .join(&category.name)
                    .join(&file.source_rel),
            );
            let installed_source_path = crate::shells::deployment::deployment_source_path(
                config,
                &category.name,
                &file.source_rel,
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

            let link_target = read_link_target(&link_path).await?;

            files.push(ShellInfoFile {
                category: category.clone(),
                file: file.clone(),
                source_path,
                installed_source_path,
                rendered_path,
                link_path,
                link_target,
                status: row.status_text,
                changes: row.changes.clone(),
            });
        }
    }
    Ok(files)
}

async fn read_link_target(path: &Path) -> Result<Option<PathBuf>> {
    let is_link = tokio::fs::symlink_metadata(path)
        .await
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok(None);
    }
    let target = tokio::fs::read_link(path)
        .await
        .with_context(|| format!("reading symlink: {}", path.display()))?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shells::handle_install;
    use tokio::fs;

    #[cfg(windows)]
    #[tokio::test]
    async fn renamed_docker_category_has_no_legacy_alias() {
        let dir = std::env::temp_dir().join(format!("shine-info-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let config = Config::new_for_test(&dir);
        tokio::fs::create_dir_all(config.shine_dir()).await.unwrap();

        let files = collect_app_files(&config).await.unwrap();
        assert!(files.iter().all(|file| file.category.name != "docker"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn shell_info_excludes_uninstalled_snapshot_siblings() {
        let dir = crate::test_support::make_temp_dir("shine-info-shell").await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"one.sh\"\ntarget = \"one\"\n\n[[files]]\nsource = \"two.sh\"\ntarget = \"two\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("one.sh"), b"#!/bin/sh\necho one\n")
            .await
            .unwrap();
        fs::write(category.join("two.sh"), b"#!/bin/sh\necho two\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom/one"), false)
            .await
            .unwrap();

        let files = collect_shell_files(&config).await.unwrap();
        assert!(files.iter().any(|file| file.file.command_name == "one"));
        assert!(files.iter().all(|file| file.file.command_name != "two"));

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
