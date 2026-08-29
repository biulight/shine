use crate::apps::{AppCategory, AppFile};
use crate::config::Config;
use crate::env::EnvConfig;
use crate::shells::metadata::ShellCategory;
use crate::status::{FileStatus, UpdateChange};
use anyhow::Result;
use std::path::PathBuf;
use utils::runtime::NullObserver;

#[derive(Clone)]
pub(super) struct AppInfoFile {
    pub(super) category: AppCategory,
    pub(super) file: AppFile,
    pub(super) destination: PathBuf,
    pub(super) status: FileStatus,
    pub(super) manifest_entry: Option<crate::install_core::AppEntry>,
    pub(super) desired_content: Option<Vec<u8>>,
    pub(super) current_content: Option<Vec<u8>>,
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
    pub(super) desired_content: Option<Vec<u8>>,
    pub(super) current_content: Option<Vec<u8>>,
    pub(super) status: &'static str,
    pub(super) changes: Vec<UpdateChange>,
}

pub(super) async fn collect_app_files(config: &Config) -> Result<Vec<AppInfoFile>> {
    let mut runtime = crate::core_runtime::from_config(config)?;
    let env = EnvConfig::load_or_init(config).await.ok();
    if let Some(env) = env {
        runtime.context_mut_for_cli().env = env.as_map().clone();
    }
    let inspections = runtime.inspect_apps(&mut NullObserver).await?;
    Ok(inspections
        .into_iter()
        .filter(|file| file.status != FileStatus::NotInstalled)
        .filter_map(|file| {
            Some(AppInfoFile {
                category: file.category,
                file: file.file,
                destination: file.destination?,
                status: file.status,
                manifest_entry: file.manifest_entry,
                desired_content: file.desired_content,
                current_content: file.current_content,
                changes: file.changes,
            })
        })
        .collect())
}

pub(super) async fn collect_shell_files(config: &Config) -> Result<Vec<ShellInfoFile>> {
    let mut runtime = crate::core_runtime::from_config(config)?;
    if let Ok(env) = EnvConfig::load_or_init(config).await {
        runtime.context_mut_for_cli().env = env.as_map().clone();
    }
    Ok(runtime
        .inspect_shells()
        .await?
        .into_iter()
        .filter(|file| file.installed)
        .map(|file| ShellInfoFile {
            category: file.category,
            file: file.file,
            source_path: file.source_path,
            installed_source_path: file.installed_source_path,
            rendered_path: file.rendered_path,
            link_path: file.link_path,
            link_target: file.link_target,
            desired_content: file.desired_content,
            current_content: file.current_content,
            status: file.status_text,
            changes: file.changes,
        })
        .collect())
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
