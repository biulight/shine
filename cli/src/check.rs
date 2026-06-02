use crate::apps::{
    AppCategory, AppListMode, AppManifest, apply_transforms, installed_content_hash,
    resolve_install_destination, source_hash_for_file,
};
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::path_display;
use anyhow::Result;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

// ---------------------------------------------------------------------------
// Shared row types
// ---------------------------------------------------------------------------

/// Per-file status used for aggregation within a category.
/// Higher discriminant = higher priority (wins in fold).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum FileStatus {
    NotInstalled,
    UpToDate,
    UpdateAvail,
    Partial,
    UserModified,
    Missing,
}

pub(crate) struct ShellRow {
    pub(crate) symbol: String,
    pub(crate) label: String,
    pub(crate) status_sym: &'static str,
    pub(crate) status_text: &'static str,
    /// `true` when at least one of preset-file or bin-symlink exists.
    pub(crate) is_installed: bool,
}

pub(crate) struct AppRow {
    pub(crate) sym: &'static str,
    pub(crate) label: String,
    pub(crate) simple_label: String,
    pub(crate) dest: Option<String>,
    pub(crate) status_text: &'static str,
    pub(crate) file_status: FileStatus,
}

// ---------------------------------------------------------------------------
// Shared row builders (data-only, no printing)
// ---------------------------------------------------------------------------

/// Build shell preset rows.  Does not include the PATH sentinel line.
pub(crate) async fn build_shell_rows(config: &Config) -> Result<Vec<ShellRow>> {
    let categories = if config.is_external_presets {
        crate::shells::metadata::load_installed_categories(config, None).await?
    } else {
        crate::shells::metadata::load_embedded_categories(None)?
    };
    if categories.is_empty() {
        return Ok(Vec::new());
    }

    let presets_shell = config.presets_dir().join("shell");
    let bin_dir = config.bin_dir();
    let mut rows: Vec<ShellRow> = Vec::new();

    for cat in &categories {
        for script in &cat.files {
            let script_path = presets_shell.join(&cat.name).join(&script.source_rel);
            let source_key = format!("shell/{}/{}", cat.name, script.source_rel.display());
            let rendered_path = config
                .rendered_dir()
                .join("shell")
                .join(&cat.name)
                .join(&script.source_rel);
            let link_name = OsString::from(&script.command_name);
            let link_path = crate::bin_links::command_path_for_name(bin_dir, &link_name);

            let file_exists = script_path.exists();
            let link_exists = link_path.exists() || {
                tokio::fs::symlink_metadata(&link_path)
                    .await
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            };

            let (sym, status_text) = match (file_exists, link_exists) {
                (true, true) => ("✓", "up-to-date"),
                (true, false) => ("~", "preset present, bin symlink missing"),
                (false, true) => ("~", "bin symlink present, preset missing"),
                (false, false) => ("✗", "not installed"),
            };

            let (sym, status_text) = match shell_template_status(
                config,
                &source_key,
                &script_path,
                &rendered_path,
            )
            .await
            {
                Some(FileStatus::UpdateAvail) if file_exists || link_exists => {
                    ("↑", "update available")
                }
                Some(FileStatus::Missing) if link_exists => ("!", "rendered script missing"),
                _ => (sym, status_text),
            };

            rows.push(ShellRow {
                symbol: colors::symbol(sym),
                label: format!("{}/{}", cat.name, script.command_name),
                status_sym: sym,
                status_text,
                is_installed: file_exists || link_exists,
            });
        }
    }

    Ok(rows)
}

async fn shell_template_status(
    config: &Config,
    source_key: &str,
    script_path: &Path,
    rendered_path: &Path,
) -> Option<FileStatus> {
    let source_bytes = if config.is_external_presets {
        tokio::fs::read(script_path).await.ok()?
    } else {
        crate::presets::read_asset_bytes(source_key)?
    };
    if !crate::presets::parse_template_annotation(&source_bytes) {
        return None;
    }

    if !rendered_path.exists() {
        return Some(FileStatus::Missing);
    }

    let env = EnvConfig::load_or_init(config).await.ok()?;
    let rendered = apply_transforms(&["template".to_string()], &source_bytes, env.as_map()).ok()?;
    let current = tokio::fs::read(rendered_path).await.ok()?;

    if rendered == current {
        Some(FileStatus::UpToDate)
    } else {
        Some(FileStatus::UpdateAvail)
    }
}

/// Build app config rows for the given pre-loaded categories.
pub(crate) async fn build_app_rows(
    config: &Config,
    categories: &[AppCategory],
) -> Result<Vec<AppRow>> {
    let manifest = AppManifest::load(config.shine_dir()).await?;
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);
    let mut rows: Vec<AppRow> = Vec::new();

    for cat in categories {
        if cat.has_explicit_files && cat.list_mode == AppListMode::Files {
            for file in &cat.files {
                let (dest_opt, status) =
                    app_file_row_status(config, cat, file, &manifest, env_map).await;

                let label = file
                    .display_name
                    .clone()
                    .unwrap_or_else(|| format!("{}/{}", cat.name, file.source_rel.display()));
                let simple_label = if cat.files.len() == 1 {
                    cat.name.clone()
                } else {
                    label.clone()
                };

                let dest_str = dest_opt.map(|d| path_display::format_home(&d, &config.home_dir));

                let (sym, status_text) = match status {
                    FileStatus::Missing => ("!", "destination missing"),
                    FileStatus::UserModified => ("~", "user modified"),
                    FileStatus::UpdateAvail => ("↑", "update available"),
                    FileStatus::UpToDate => ("✓", "up-to-date"),
                    FileStatus::NotInstalled | FileStatus::Partial => ("✗", "not installed"),
                };

                rows.push(AppRow {
                    sym,
                    label,
                    simple_label,
                    dest: dest_str,
                    status_text,
                    file_status: status,
                });
            }
        } else {
            let mut file_statuses: Vec<FileStatus> = Vec::new();

            for file in &cat.files {
                let (_, status) = app_file_row_status(config, cat, file, &manifest, env_map).await;
                file_statuses.push(status);
            }

            let has_installed = file_statuses.iter().any(|s| {
                matches!(
                    s,
                    FileStatus::UpToDate | FileStatus::UpdateAvail | FileStatus::UserModified
                )
            });
            let has_not_installed = file_statuses.contains(&FileStatus::NotInstalled);
            let cat_status = if has_installed && has_not_installed {
                // Use the max status of installed files. Only collapse to Partial
                // when all installed files are up-to-date; higher-severity statuses
                // (UpdateAvail, UserModified) take priority because the user action
                // ("shine upgrade") handles updates for installed files.
                let installed_max = file_statuses
                    .iter()
                    .copied()
                    .filter(|s| *s != FileStatus::NotInstalled)
                    .max()
                    .unwrap_or(FileStatus::Partial);
                if installed_max == FileStatus::UpToDate {
                    FileStatus::Partial
                } else {
                    installed_max
                }
            } else {
                file_statuses
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(FileStatus::NotInstalled)
            };

            let dest_display: Option<String> = if let Some(root) = &cat.destination_root {
                Some(path_display::format_tilde_path(root, &config.home_dir))
            } else if cat.files.len() == 1 {
                resolve_install_destination(cat, &cat.files[0], config)
                    .ok()
                    .map(|p| path_display::format_home(&p, &config.home_dir))
            } else {
                None
            };

            let (sym, status_text) = match cat_status {
                FileStatus::Missing => ("!", "destination missing"),
                FileStatus::UserModified => ("~", "user modified"),
                FileStatus::Partial => ("~", "partial install"),
                FileStatus::UpdateAvail => ("↑", "update available"),
                FileStatus::UpToDate => ("✓", "up-to-date"),
                FileStatus::NotInstalled => ("✗", "not installed"),
            };

            rows.push(AppRow {
                sym,
                label: cat.name.clone(),
                simple_label: cat.name.clone(),
                dest: dest_display,
                status_text,
                file_status: cat_status,
            });
        }
    }

    Ok(rows)
}

async fn app_file_row_status(
    config: &Config,
    cat: &AppCategory,
    file: &crate::apps::AppFile,
    manifest: &AppManifest,
    env: &BTreeMap<String, String>,
) -> (Option<std::path::PathBuf>, FileStatus) {
    match resolve_install_destination(cat, file, config) {
        Err(_) => (None, FileStatus::NotInstalled),
        Ok(dest) => {
            let status = match manifest.find_by_dest(&dest) {
                None => FileStatus::NotInstalled,
                Some(entry) => {
                    if !dest.exists() {
                        FileStatus::Missing
                    } else {
                        match tokio::fs::read(&dest).await {
                            Err(_) => FileStatus::Missing,
                            Ok(dest_bytes) => {
                                let manifest_hash = entry.content_hash;
                                match installed_content_hash(file, &dest_bytes) {
                                    Ok(Some(dest_hash)) if dest_hash == manifest_hash => {
                                        match source_hash_for_file(config, cat, file, env).await {
                                            Some(src) if src != manifest_hash => {
                                                FileStatus::UpdateAvail
                                            }
                                            _ => FileStatus::UpToDate,
                                        }
                                    }
                                    Ok(None) => FileStatus::Missing,
                                    Ok(Some(_)) | Err(_) => FileStatus::UserModified,
                                }
                            }
                        }
                    }
                }
            };
            (Some(dest), status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{AppFile, AppInstallStrategy};
    use crate::config::Config;
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::sync::{Mutex, OnceLock};
    use tokio::fs;

    #[cfg(windows)]
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock must not be poisoned")
    }

    async fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-check-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn installed_shell_rows_use_windows_shim_path() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.ps1\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("set_proxy.ps1"), b"Write-Output proxy\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        fs::write(config.bin_dir().join("setproxy.ps1"), b"# shine-managed\n")
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "proxy/setproxy")
            .expect("proxy/setproxy row should exist");

        assert_eq!(row.status_sym, "✓");
        assert_eq!(row.status_text, "up-to-date");
        assert!(row.is_installed);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn installed_shell_rows_report_up_to_date() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(&script, b"#!/bin/bash\necho proxy\n")
            .await
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).await.unwrap();
        }

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        crate::shells::handle_install(&config, Some("proxy"), false)
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "proxy/setproxy")
            .expect("proxy/setproxy row should exist");

        assert_eq!(row.status_sym, "✓");
        assert_eq!(row.status_text, "up-to-date");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_template_shell_change_reports_update_available() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(
            &script,
            b"#!/bin/bash\n# shine-template: true\necho @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        crate::shells::handle_install(&config, Some("proxy"), false)
            .await
            .unwrap();

        fs::write(
            &script,
            b"#!/bin/bash\n# shine-template: true\necho changed @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "proxy/setproxy")
            .expect("proxy/setproxy row should exist");

        assert_eq!(row.status_sym, "↑");
        assert_eq!(row.status_text, "update available");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn category_list_mode_aggregates_explicit_app_files() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let category = AppCategory {
            name: "ghostty".to_string(),
            description: Some("Ghostty terminal configuration.".to_string()),
            destination_root: Some(dir.join(".config/ghostty").display().to_string()),
            files: vec![
                AppFile {
                    source_rel: PathBuf::from("config.ghostty"),
                    target_rel: PathBuf::from("config.ghostty"),
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                },
                AppFile {
                    source_rel: PathBuf::from("themes/shine-light"),
                    target_rel: PathBuf::from("themes/shine-light"),
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec!["template".to_string()],
                    install_strategy: AppInstallStrategy::Copy,
                },
            ],
            list_mode: AppListMode::Category,
            uses_metadata: true,
            has_explicit_files: true,
        };

        let rows = build_app_rows(&config, &[category]).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "ghostty");
        assert_eq!(rows[0].simple_label, "ghostty");
        assert_eq!(rows[0].dest.as_deref(), Some("~/.config/ghostty"));
        assert_eq!(rows[0].file_status, FileStatus::NotInstalled);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn file_list_mode_keeps_file_labels_for_multi_file_app_simple_list() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let category = AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: Some(dir.join(".config/sample").display().to_string()),
            files: vec![
                AppFile {
                    source_rel: PathBuf::from("config.toml"),
                    target_rel: PathBuf::from("config.toml"),
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                },
                AppFile {
                    source_rel: PathBuf::from("theme.toml"),
                    target_rel: PathBuf::from("theme.toml"),
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                },
            ],
            list_mode: AppListMode::Files,
            uses_metadata: true,
            has_explicit_files: true,
        };

        let rows = build_app_rows(&config, &[category]).await.unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "sample/config.toml");
        assert_eq!(rows[0].simple_label, "sample/config.toml");
        assert_eq!(rows[1].label, "sample/theme.toml");
        assert_eq!(rows[1].simple_label, "sample/theme.toml");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_docker_engine_row_uses_engine_destination() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: env_lock() serialises all Windows env-mutation tests, preventing
        // concurrent writes to the process environment from other test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let categories = crate::apps::load_embedded_categories(Some("docker-engine")).unwrap();
        let rows = build_app_rows(&config, &categories).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "docker-engine/daemon.jsonc");
        assert_eq!(rows[0].simple_label, "docker-engine");
        assert_eq!(rows[0].file_status, FileStatus::NotInstalled);
        assert_eq!(rows[0].dest.as_deref(), Some("~/.docker/daemon.json"));

        // SAFETY: same env_lock() guard as above.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_docker_desktop_row_uses_forward_slash_destination() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: env_lock() serialises all Windows env-mutation tests, preventing
        // concurrent writes to the process environment from other test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let categories = crate::apps::load_embedded_categories(Some("docker-desktop")).unwrap();
        let rows = build_app_rows(&config, &categories).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "docker-desktop/settings-store.jsonc");
        assert_eq!(rows[0].simple_label, "docker-desktop");
        assert_eq!(rows[0].file_status, FileStatus::NotInstalled);
        assert_eq!(
            rows[0].dest.as_deref(),
            Some("~/AppData/Roaming/Docker/settings-store.json")
        );

        // SAFETY: same env_lock() guard as above.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
