//! Shared install-status row builders consumed by `list` and `info`.
//!
//! Not a routed command itself (`shine check` was removed) — this is a
//! status-row library: it computes per-file/per-category install status
//! (`FileStatus`) and renders it into `AppRow`/`ShellRow` for display.

use crate::apps::{AppCategory, AppListMode};
#[cfg(test)]
use crate::apps::{installed_content_hash, resolve_install_destination, source_hash_for_file};
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
#[cfg(test)]
use crate::install_core::{AppEntry, AppManifest};
use crate::path_display;
use anyhow::Result;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::path::PathBuf;
use utils::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};

// ---------------------------------------------------------------------------
// Shared row types
// ---------------------------------------------------------------------------

pub(crate) use utils::runtime::InspectionChange as UpdateChange;
pub use utils::runtime::InspectionFileStatus as FileStatus;

#[cfg(test)]
pub(crate) struct AppFileAssessment {
    pub(crate) destination: Option<PathBuf>,
    pub(crate) status: FileStatus,
    pub(crate) changes: Vec<UpdateChange>,
}

pub struct ShellRow {
    /// Shell preset category owning this command row. Lifecycle commands act
    /// on this category; `label` remains the command-level diagnostic target.
    pub category: String,
    pub symbol: String,
    pub label: String,
    pub status_sym: &'static str,
    pub status_text: &'static str,
    /// `true` when at least one of preset-file or bin-symlink exists.
    pub is_installed: bool,
    /// Existing launcher is outside Shine's ownership proof and must be
    /// preserved rather than reported as an applicable update.
    pub(crate) link_conflict: bool,
    pub(crate) changes: Vec<UpdateChange>,
}

pub struct AppRow {
    /// App preset category owning this row. Unlike `label`, this is stable
    /// even when a multi-file category supplies custom display names.
    pub category: String,
    pub sym: &'static str,
    pub label: String,
    pub simple_label: String,
    pub dest: Option<String>,
    pub status_text: &'static str,
    pub file_status: FileStatus,
}

// ---------------------------------------------------------------------------
// Shared row builders (data-only, no printing)
// ---------------------------------------------------------------------------

/// Build shell preset rows.  Does not include the PATH sentinel line.
pub async fn build_shell_rows(config: &Config) -> Result<Vec<ShellRow>> {
    let inspections = crate::core_runtime::from_config(config)
        .await?
        .inspect_shells()
        .await?;
    Ok(inspections
        .into_iter()
        .map(|file| {
            let (symbol, status_sym) = match file.status {
                FileStatus::NotInstalled => ("✗", "✗"),
                FileStatus::UpdateAvail => ("↑", "↑"),
                FileStatus::Missing => ("!", "!"),
                FileStatus::Partial | FileStatus::UserModified => ("~", "~"),
                FileStatus::UpToDate => ("✓", "✓"),
            };
            ShellRow {
                category: file.category.name.clone(),
                symbol: colors::symbol(symbol),
                label: format!("{}/{}", file.category.name, file.file.command_name),
                status_sym,
                status_text: file.status_text,
                is_installed: file.installed,
                link_conflict: file.link_conflict,
                changes: file.changes,
            }
        })
        .collect())
}
pub async fn build_app_rows(config: &Config, categories: &[AppCategory]) -> Result<Vec<AppRow>> {
    build_app_rows_with_lifecycle(config, categories)
        .await
        .map(|(rows, _)| rows)
}

pub(crate) async fn build_app_rows_with_lifecycle(
    config: &Config,
    categories: &[AppCategory],
) -> Result<(Vec<AppRow>, LifecycleResultV1)> {
    let mut runtime = crate::core_runtime::from_config(config).await?;
    if let Ok(env) = EnvConfig::load_or_init(config).await {
        runtime.context_mut_for_cli().env = env.as_map().clone();
    }
    let selected = categories
        .iter()
        .map(|category| category.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let inspections = runtime
        .inspect_apps(&mut utils::runtime::NullObserver)
        .await?
        .into_iter()
        .filter(|file| selected.contains(file.category.name.as_str()))
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);

    for category in categories {
        let files = inspections
            .iter()
            .filter(|file| file.category.name == category.name)
            .collect::<Vec<_>>();
        for inspection in &files {
            let manifest_owned = inspection.manifest_entry.is_some()
                || inspection
                    .changes
                    .iter()
                    .any(|change| matches!(change, UpdateChange::NewFile { .. }));
            if manifest_owned {
                let target = format!("app/{}", category.name);
                let resource = Some(inspection.file.source_rel.display().to_string());
                let outcome = match inspection.status {
                    FileStatus::UpToDate => Some(LifecycleOutcomeV1::new(
                        target,
                        resource,
                        LifecycleStatus::Unchanged,
                        [],
                    )),
                    FileStatus::UpdateAvail => {
                        let relocated = inspection.changes.iter().any(|change| {
                            matches!(change, UpdateChange::DestinationRelocated { .. })
                        });
                        let mut effects = Vec::new();
                        if relocated {
                            effects.push(LifecycleEffect::ResourceRemovePreviewed);
                        }
                        effects.push(LifecycleEffect::ResourceWritePreviewed);
                        effects.push(LifecycleEffect::ReceiptWritePreviewed);
                        Some(LifecycleOutcomeV1::new(
                            target,
                            resource,
                            LifecycleStatus::Pending,
                            effects,
                        ))
                    }
                    FileStatus::Missing => Some(LifecycleOutcomeV1::new(
                        target,
                        resource,
                        LifecycleStatus::Pending,
                        [
                            LifecycleEffect::ResourceWritePreviewed,
                            LifecycleEffect::ReceiptWritePreviewed,
                        ],
                    )),
                    FileStatus::UserModified => Some(
                        LifecycleOutcomeV1::new(
                            target,
                            resource,
                            LifecycleStatus::Conflict,
                            [LifecycleEffect::UserResourcePreserved],
                        )
                        .with_diagnostic_code("app_user_modified"),
                    ),
                    FileStatus::NotInstalled | FileStatus::Partial => None,
                };
                if let Some(outcome) = outcome {
                    lifecycle.push(outcome);
                }
            }
        }

        if category.has_explicit_files && category.list_mode == AppListMode::Files {
            for inspection in files {
                let label = inspection.file.display_name.clone().unwrap_or_else(|| {
                    format!("{}/{}", category.name, inspection.file.source_rel.display())
                });
                let simple_label = if category.files.len() == 1 {
                    category.name.clone()
                } else {
                    label.clone()
                };
                let (sym, status_text) = app_status_presentation(inspection.status);
                rows.push(AppRow {
                    category: category.name.clone(),
                    sym,
                    label,
                    simple_label,
                    dest: inspection
                        .destination
                        .as_ref()
                        .map(|path| path_display::format_home(path, &config.home_dir)),
                    status_text,
                    file_status: inspection.status,
                });
            }
        } else {
            let statuses = files.iter().map(|file| file.status).collect::<Vec<_>>();
            let has_installed = statuses.iter().any(|status| {
                matches!(
                    status,
                    FileStatus::UpToDate | FileStatus::UpdateAvail | FileStatus::UserModified
                )
            });
            let has_not_installed = statuses.contains(&FileStatus::NotInstalled);
            let status = if has_installed && has_not_installed {
                let installed_max = statuses
                    .iter()
                    .copied()
                    .filter(|status| *status != FileStatus::NotInstalled)
                    .max()
                    .unwrap_or(FileStatus::Partial);
                if installed_max == FileStatus::UpToDate {
                    FileStatus::Partial
                } else {
                    installed_max
                }
            } else {
                statuses
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(FileStatus::NotInstalled)
            };
            let destination = if let Some(root) = &category.destination_root {
                Some(path_display::format_tilde_path(root, &config.home_dir))
            } else if files.len() == 1 {
                files[0]
                    .destination
                    .as_ref()
                    .map(|path| path_display::format_home(path, &config.home_dir))
            } else {
                None
            };
            let (sym, status_text) = app_status_presentation(status);
            rows.push(AppRow {
                category: category.name.clone(),
                sym,
                label: category.name.clone(),
                simple_label: category.name.clone(),
                dest: destination,
                status_text: if status == FileStatus::Partial {
                    "partial install"
                } else {
                    status_text
                },
                file_status: status,
            });
        }
    }
    Ok((rows, lifecycle))
}

fn app_status_presentation(status: FileStatus) -> (&'static str, &'static str) {
    match status {
        FileStatus::Missing => ("!", "destination missing"),
        FileStatus::UserModified => ("~", "user modified"),
        FileStatus::UpdateAvail => ("↑", "update available"),
        FileStatus::UpToDate => ("✓", "up-to-date"),
        FileStatus::NotInstalled | FileStatus::Partial => ("✗", "not installed"),
    }
}

#[cfg(test)]
fn app_update_outcome(
    category: &AppCategory,
    file: &crate::apps::AppFile,
    assessment: &AppFileAssessment,
    manifest: &AppManifest,
) -> Option<LifecycleOutcomeV1> {
    let source = format!("app/{}/{}", category.name, file.source_rel.display());
    let owned = manifest.find_by_source(&source).is_some()
        || assessment
            .destination
            .as_ref()
            .is_some_and(|destination| manifest.find_by_dest(destination).is_some())
        || assessment
            .changes
            .iter()
            .any(|change| matches!(change, UpdateChange::NewFile { .. }));
    if !owned {
        return None;
    }
    let target = format!("app/{}", category.name);
    let resource = Some(file.source_rel.display().to_string());
    match assessment.status {
        FileStatus::UpToDate => Some(LifecycleOutcomeV1::new(
            target,
            resource,
            LifecycleStatus::Unchanged,
            [],
        )),
        FileStatus::UpdateAvail => {
            let mut effects = Vec::new();
            if assessment
                .changes
                .iter()
                .any(|change| matches!(change, UpdateChange::DestinationRelocated { .. }))
            {
                effects.push(LifecycleEffect::ResourceRemovePreviewed);
            }
            effects.extend([
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ]);
            Some(LifecycleOutcomeV1::new(
                target,
                resource,
                LifecycleStatus::Pending,
                effects,
            ))
        }
        FileStatus::Missing => Some(LifecycleOutcomeV1::new(
            target,
            resource,
            LifecycleStatus::Pending,
            [
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ],
        )),
        FileStatus::UserModified => Some(
            LifecycleOutcomeV1::new(
                target,
                resource,
                LifecycleStatus::Conflict,
                [LifecycleEffect::UserResourcePreserved],
            )
            .with_diagnostic_code("app_user_modified"),
        ),
        FileStatus::NotInstalled | FileStatus::Partial => None,
    }
}
#[cfg(test)]
pub(crate) async fn app_file_row_status(
    config: &Config,
    cat: &AppCategory,
    file: &crate::apps::AppFile,
    manifest: &AppManifest,
    env: &BTreeMap<String, String>,
) -> (Option<std::path::PathBuf>, FileStatus) {
    let assessment = assess_app_file(config, cat, file, manifest, env).await;
    (assessment.destination, assessment.status)
}

#[cfg(test)]
pub(crate) async fn assess_app_file(
    config: &Config,
    cat: &AppCategory,
    file: &crate::apps::AppFile,
    manifest: &AppManifest,
    env: &BTreeMap<String, String>,
) -> AppFileAssessment {
    match resolve_install_destination(cat, file, config) {
        Err(_) => AppFileAssessment {
            destination: None,
            status: FileStatus::NotInstalled,
            changes: Vec::new(),
        },
        Ok(dest) => {
            let source = format!("app/{}/{}", cat.name, file.source_rel.display());
            let installed_category = manifest.entries.iter().any(|entry| {
                entry
                    .source
                    .strip_prefix("app/")
                    .and_then(|source| source.split_once('/'))
                    .is_some_and(|(category, _)| category == cat.name)
            });
            let mut changes = Vec::new();
            let status = match manifest.find_by_dest(&dest) {
                Some(entry) => {
                    let status = app_entry_status(config, cat, file, entry, env).await;
                    if status == FileStatus::UpdateAvail {
                        changes.push(UpdateChange::ContentChanged);
                    }
                    status
                }
                None => match manifest.find_by_source(&source) {
                    Some(entry)
                        if file
                            .generator
                            .as_ref()
                            .is_some_and(|generator| !generator.auto) =>
                    {
                        return AppFileAssessment {
                            destination: Some(entry.destination.clone()),
                            status: app_entry_status(config, cat, file, entry, env).await,
                            changes: Vec::new(),
                        };
                    }
                    Some(entry) => {
                        changes.push(UpdateChange::DestinationRelocated {
                            from: entry.destination.clone(),
                            to: dest.clone(),
                        });
                        if file
                            .generator
                            .as_ref()
                            .is_none_or(|generator| generator.auto)
                            && source_hash_for_file(config, cat, file, env)
                                .await
                                .is_some_and(|hash| hash != entry.content_hash)
                        {
                            changes.push(UpdateChange::ContentChanged);
                        }
                        FileStatus::UpdateAvail
                    }
                    None if installed_category
                        && file
                            .generator
                            .as_ref()
                            .is_none_or(|generator| generator.auto) =>
                    {
                        if source_hash_for_file(config, cat, file, env).await.is_some() {
                            changes.push(UpdateChange::NewFile {
                                destination: dest.clone(),
                            });
                            FileStatus::UpdateAvail
                        } else {
                            FileStatus::NotInstalled
                        }
                    }
                    None => FileStatus::NotInstalled,
                },
            };
            AppFileAssessment {
                destination: Some(dest),
                status,
                changes,
            }
        }
    }
}

/// Computes the status of an already-resolved manifest entry: compares its
/// recorded content hash against what's currently on disk at
/// `entry.destination`, and (if unchanged) against the current preset
/// source to detect an available update.
///
/// Shared by `app_file_row_status` (used by `list`/`app info`) and `info`'s
/// `collect_app_files` — both need this exact computation once an `AppEntry`
/// has been resolved.
#[cfg(test)]
pub(crate) async fn app_entry_status(
    config: &Config,
    cat: &AppCategory,
    file: &crate::apps::AppFile,
    entry: &AppEntry,
    env: &BTreeMap<String, String>,
) -> FileStatus {
    // Generators are intentionally polled on every status/update pass, even
    // when the installed destination was edited. Static sources keep the
    // cheaper existing behavior and are read only after ownership is proven.
    let generator_enabled = file
        .generator
        .as_ref()
        .is_some_and(|generator| generator.auto && env.contains_key(&generator.when_env));
    let manual_generator = file
        .generator
        .as_ref()
        .is_some_and(|generator| !generator.auto);
    let generated_source_hash = if generator_enabled {
        source_hash_for_file(config, cat, file, env).await
    } else {
        None
    };
    if !entry.destination.exists() {
        return FileStatus::Missing;
    }
    match tokio::fs::read(&entry.destination).await {
        Err(_) => FileStatus::Missing,
        Ok(dest_bytes) => {
            let manifest_hash = entry.content_hash;
            match installed_content_hash(file, &dest_bytes) {
                Ok(Some(dest_hash)) if dest_hash == manifest_hash => {
                    if manual_generator {
                        return FileStatus::UpToDate;
                    }
                    let source_hash = if generator_enabled {
                        generated_source_hash
                    } else {
                        source_hash_for_file(config, cat, file, env).await
                    };
                    match source_hash {
                        Some(src) if src != manifest_hash => FileStatus::UpdateAvail,
                        _ => FileStatus::UpToDate,
                    }
                }
                Ok(None) => FileStatus::Missing,
                Ok(Some(_)) | Err(_) => FileStatus::UserModified,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppFile;
    use crate::config::Config;
    use crate::install_core::AppInstallStrategy;
    #[cfg(windows)]
    use crate::test_support::env_lock;
    use std::path::{Path, PathBuf};
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-check").await
    }

    fn sample_app_file() -> AppFile {
        AppFile {
            source_rel: PathBuf::from("dest.txt"),
            target_rel: PathBuf::from("dest.txt"),
            destination_root: None,
            description: None,
            display_name: None,
            legacy_dest_annotation: None,
            transforms: vec![],
            install_strategy: AppInstallStrategy::Copy,
            requires_admin: false,
            restart_hint: None,
            generator: None,
        }
    }

    fn sample_app_category() -> AppCategory {
        AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: None,
            files: vec![sample_app_file()],
            list_mode: AppListMode::Files,
            post_upgrade: Vec::new(),
            post_install: Vec::new(),
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
        }
    }

    fn sample_app_entry(destination: PathBuf, content_hash: u64) -> AppEntry {
        AppEntry {
            source: "app/sample/dest.txt".to_string(),
            destination,
            backup: None,
            content_hash,
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    }

    #[test]
    fn app_update_outcomes_map_owned_conflicts_missing_files_and_relocations() {
        let destination = PathBuf::from("/private/machine/dest.txt");
        let manifest = AppManifest {
            entries: vec![sample_app_entry(destination.clone(), 1)],
            ..AppManifest::default()
        };
        let category = sample_app_category();
        let file = sample_app_file();

        let missing = app_update_outcome(
            &category,
            &file,
            &AppFileAssessment {
                destination: Some(destination.clone()),
                status: FileStatus::Missing,
                changes: Vec::new(),
            },
            &manifest,
        )
        .unwrap();
        assert_eq!(missing.status, LifecycleStatus::Pending);
        assert_eq!(
            missing.effects,
            [
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ]
        );

        let conflict = app_update_outcome(
            &category,
            &file,
            &AppFileAssessment {
                destination: Some(destination.clone()),
                status: FileStatus::UserModified,
                changes: Vec::new(),
            },
            &manifest,
        )
        .unwrap();
        assert_eq!(conflict.status, LifecycleStatus::Conflict);
        assert_eq!(conflict.effects, [LifecycleEffect::UserResourcePreserved]);
        assert_eq!(conflict.diagnostic_codes, ["app_user_modified"]);

        let relocated = app_update_outcome(
            &category,
            &file,
            &AppFileAssessment {
                destination: Some(PathBuf::from("/private/machine/new.txt")),
                status: FileStatus::UpdateAvail,
                changes: vec![UpdateChange::DestinationRelocated {
                    from: destination,
                    to: PathBuf::from("/private/machine/new.txt"),
                }],
            },
            &manifest,
        )
        .unwrap();
        assert_eq!(relocated.status, LifecycleStatus::Pending);
        assert_eq!(
            relocated.effects,
            [
                LifecycleEffect::ResourceRemovePreviewed,
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ]
        );
        assert!(
            !serde_json::to_string(&relocated)
                .unwrap()
                .contains("/private/machine")
        );
    }

    #[tokio::test]
    async fn app_entry_status_reports_missing_when_destination_absent() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let dest = dir.join("dest.txt");
        let entry = sample_app_entry(dest, crate::install_core::hash_content(b"hello"));

        let status = app_entry_status(
            &config,
            &sample_app_category(),
            &sample_app_file(),
            &entry,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(status, FileStatus::Missing);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_entry_status_reports_user_modified_when_dest_hash_differs() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let dest = dir.join("dest.txt");
        fs::write(&dest, b"locally edited").await.unwrap();
        let entry = sample_app_entry(dest, crate::install_core::hash_content(b"original"));

        let status = app_entry_status(
            &config,
            &sample_app_category(),
            &sample_app_file(),
            &entry,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(status, FileStatus::UserModified);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_entry_status_reports_up_to_date_when_source_unreadable() {
        // No embedded/external source exists for the synthetic "sample"
        // category, so source_hash_for_file returns None and the status
        // falls back to UpToDate once the dest hash matches the manifest.
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let dest = dir.join("dest.txt");
        fs::write(&dest, b"hello").await.unwrap();
        let entry = sample_app_entry(dest, crate::install_core::hash_content(b"hello"));

        let status = app_entry_status(
            &config,
            &sample_app_category(),
            &sample_app_file(),
            &entry,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(status, FileStatus::UpToDate);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_entry_status_reports_update_available_when_source_changed() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let source_path = config.preset_path(Path::new("app").join("sample").join("dest.txt"));
        fs::create_dir_all(source_path.parent().unwrap())
            .await
            .unwrap();
        fs::write(&source_path, b"new upstream content")
            .await
            .unwrap();

        let dest = dir.join("dest.txt");
        fs::write(&dest, b"hello").await.unwrap();
        let entry = sample_app_entry(dest, crate::install_core::hash_content(b"hello"));

        let category = AppCategory {
            destination_root: Some(dir.display().to_string()),
            ..sample_app_category()
        };
        let assessment = assess_app_file(
            &config,
            &category,
            &sample_app_file(),
            &AppManifest {
                entries: vec![entry],
                ..AppManifest::default()
            },
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(assessment.status, FileStatus::UpdateAvail);
        assert_eq!(assessment.changes, vec![UpdateChange::ContentChanged]);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_file_row_status_reports_not_installed_without_manifest_entry() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let manifest = AppManifest::default();
        let category = AppCategory {
            destination_root: Some(dir.display().to_string()),
            ..sample_app_category()
        };

        let (dest, status) = app_file_row_status(
            &config,
            &category,
            &sample_app_file(),
            &manifest,
            &BTreeMap::new(),
        )
        .await;

        assert!(dest.is_some());
        assert_eq!(status, FileStatus::NotInstalled);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_file_row_status_reports_new_file_in_installed_category_as_update() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let source_dir = config.preset_path(Path::new("app/sample"));
        fs::create_dir_all(&source_dir).await.unwrap();
        fs::write(source_dir.join("new.txt"), b"new").await.unwrap();

        let mut file = sample_app_file();
        file.source_rel = PathBuf::from("new.txt");
        file.target_rel = PathBuf::from("new.txt");
        let category = AppCategory {
            destination_root: Some(dir.join("dest").display().to_string()),
            files: vec![file.clone()],
            ..sample_app_category()
        };
        let manifest = AppManifest {
            entries: vec![sample_app_entry(
                dir.join("dest/old.txt"),
                crate::install_core::hash_content(b"old"),
            )],
            ..AppManifest::default()
        };

        let assessment =
            assess_app_file(&config, &category, &file, &manifest, &BTreeMap::new()).await;

        assert_eq!(assessment.status, FileStatus::UpdateAvail);
        assert_eq!(
            assessment.changes,
            vec![UpdateChange::NewFile {
                destination: dir.join("dest/new.txt")
            }]
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_file_row_status_reports_destination_move_as_update() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let source_dir = config.preset_path(Path::new("app/sample"));
        fs::create_dir_all(&source_dir).await.unwrap();
        fs::write(source_dir.join("dest.txt"), b"managed")
            .await
            .unwrap();

        let old_destination = dir.join("old/dest.txt");
        let category = AppCategory {
            destination_root: Some(dir.join("new").display().to_string()),
            ..sample_app_category()
        };
        let manifest = AppManifest {
            entries: vec![sample_app_entry(
                old_destination,
                crate::install_core::hash_content(b"managed"),
            )],
            ..AppManifest::default()
        };

        let assessment = assess_app_file(
            &config,
            &category,
            &category.files[0],
            &manifest,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(assessment.status, FileStatus::UpdateAvail);
        assert_eq!(
            assessment.changes,
            vec![UpdateChange::DestinationRelocated {
                from: dir.join("old/dest.txt"),
                to: dir.join("new/dest.txt"),
            }]
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn manual_generator_destination_move_preserves_installed_snapshot() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let source_dir = config.preset_path(Path::new("app/sample"));
        fs::create_dir_all(&source_dir).await.unwrap();
        fs::write(source_dir.join("dest.txt"), b"static fallback")
            .await
            .unwrap();
        fs::write(source_dir.join("generate.sh"), b"#!/bin/sh\n")
            .await
            .unwrap();
        fs::write(
            source_dir.join("shine.toml"),
            format!(
                "dest = {:?}\n\n[[files]]\nsource = \"dest.txt\"\ntarget = \"dest.txt\"\ngenerator = {{ script = \"generate.sh\", env = [\"SOURCE_URL\"], when_env = \"SOURCE_URL\", auto = false }}\n",
                dir.join("new").display().to_string()
            ),
        )
        .await
        .unwrap();

        let old_destination = dir.join("old/dest.txt");
        fs::create_dir_all(old_destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(&old_destination, b"generated snapshot")
            .await
            .unwrap();

        let mut categories = crate::apps::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let category = categories.remove(0);
        let file = category.files[0].clone();
        let manifest = AppManifest {
            entries: vec![sample_app_entry(
                old_destination.clone(),
                crate::install_core::hash_content(b"generated snapshot"),
            )],
            ..AppManifest::default()
        };

        let assessment =
            assess_app_file(&config, &category, &file, &manifest, &BTreeMap::new()).await;

        assert_eq!(assessment.destination, Some(old_destination));
        assert_eq!(assessment.status, FileStatus::UpToDate);
        assert!(assessment.changes.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn app_destination_move_can_also_report_content_change() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let source_dir = config.preset_path(Path::new("app/sample"));
        fs::create_dir_all(&source_dir).await.unwrap();
        fs::write(source_dir.join("dest.txt"), b"new content")
            .await
            .unwrap();

        let category = AppCategory {
            destination_root: Some(dir.join("new").display().to_string()),
            ..sample_app_category()
        };
        let manifest = AppManifest {
            entries: vec![sample_app_entry(
                dir.join("old/dest.txt"),
                crate::install_core::hash_content(b"old content"),
            )],
            ..AppManifest::default()
        };

        let assessment = assess_app_file(
            &config,
            &category,
            &category.files[0],
            &manifest,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(assessment.status, FileStatus::UpdateAvail);
        assert_eq!(
            assessment.changes,
            vec![
                UpdateChange::DestinationRelocated {
                    from: dir.join("old/dest.txt"),
                    to: dir.join("new/dest.txt"),
                },
                UpdateChange::ContentChanged,
            ]
        );
        fs::remove_dir_all(&dir).await.unwrap();
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

        assert_ne!(row.status_text, "not installed");
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
    async fn missing_shell_command_entry_is_an_update_reason() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("tool.sh"), b"#!/bin/sh\necho same\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        fs::remove_file(config.bin_dir().join("mytool"))
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_text, "update available");
        assert_eq!(
            row.changes,
            vec![UpdateChange::CommandEntryMissing {
                path: config.bin_dir().join("mytool"),
            }]
        );

        fs::remove_file(config.shine_dir().join("shell-manifest.toml"))
            .await
            .unwrap();
        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert!(!row.is_installed);
        assert_eq!(row.status_text, "not installed");
        assert!(row.changes.is_empty());

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

    #[cfg(unix)]
    #[tokio::test]
    async fn live_raw_shell_change_stays_live_and_current() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        let source = cat_dir.join("tool.sh");
        fs::write(&source, b"#!/bin/sh\necho first\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.external_shell_mode = crate::config::ExternalShellMode::Live;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        fs::write(&source, b"#!/bin/sh\necho second\n")
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_sym, "✓");
        assert_eq!(row.status_text, "live source");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn live_overlay_root_rename_reports_source_relocation_without_content_change() {
        let dir = make_temp_dir().await;
        let old_overlay = dir.join("shineOverlay");
        let new_overlay = dir.join("shineOverlayTest");
        let old_category = old_overlay.join("shell/custom");
        fs::create_dir_all(&old_category).await.unwrap();
        fs::write(
            old_category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        fs::write(old_category.join("tool.sh"), b"#!/bin/sh\necho same\n")
            .await
            .unwrap();

        let mut old_config =
            Config::new_for_test(&dir).with_presets_overlay_dir_override(Some(old_overlay.clone()));
        old_config.is_external_presets = true;
        old_config.external_shell_mode = crate::config::ExternalShellMode::Live;
        fs::create_dir_all(old_config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&old_config, Some("custom"), false)
            .await
            .unwrap();

        fs::rename(&old_overlay, &new_overlay).await.unwrap();
        let mut new_config =
            Config::new_for_test(&dir).with_presets_overlay_dir_override(Some(new_overlay.clone()));
        new_config.is_external_presets = true;
        new_config.external_shell_mode = crate::config::ExternalShellMode::Live;

        let rows = build_shell_rows(&new_config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_text, "update available");
        assert_eq!(
            row.changes,
            vec![UpdateChange::SourceRelocated {
                from: old_overlay.join("shell/custom/tool.sh"),
                to: new_overlay.join("shell/custom/tool.sh"),
            }]
        );

        fs::write(
            new_overlay.join("shell/custom/tool.sh"),
            b"#!/bin/sh\necho changed\n",
        )
        .await
        .unwrap();
        let rows = build_shell_rows(&new_config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(
            row.changes,
            vec![
                UpdateChange::SourceRelocated {
                    from: old_overlay.join("shell/custom/tool.sh"),
                    to: new_overlay.join("shell/custom/tool.sh"),
                },
                UpdateChange::ContentChanged,
            ]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshot_overlay_root_rename_with_same_bytes_stays_current() {
        let dir = make_temp_dir().await;
        let old_overlay = dir.join("shineOverlay");
        let new_overlay = dir.join("shineOverlayTest");
        let old_category = old_overlay.join("shell/custom");
        fs::create_dir_all(&old_category).await.unwrap();
        fs::write(
            old_category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        fs::write(old_category.join("tool.sh"), b"#!/bin/sh\necho same\n")
            .await
            .unwrap();

        let mut old_config =
            Config::new_for_test(&dir).with_presets_overlay_dir_override(Some(old_overlay.clone()));
        old_config.is_external_presets = true;
        fs::create_dir_all(old_config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&old_config, Some("custom"), false)
            .await
            .unwrap();

        fs::rename(&old_overlay, &new_overlay).await.unwrap();
        let mut new_config =
            Config::new_for_test(&dir).with_presets_overlay_dir_override(Some(new_overlay));
        new_config.is_external_presets = true;

        let rows = build_shell_rows(&new_config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_text, "up-to-date");
        assert!(row.changes.is_empty());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_manifest_metadata_changes_are_reported_field_by_field() {
        use crate::shells::deployment::{ShellManifest, ShellManifestEntry};
        use std::os::unix::fs::symlink;

        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        let source = category.join("tool.sh");
        let bytes = b"#!/bin/sh\necho same\n";
        fs::write(&source, bytes).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.external_shell_mode = crate::config::ExternalShellMode::Live;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        symlink(&source, config.bin_dir().join("mytool")).unwrap();

        ShellManifest {
            entries: vec![ShellManifestEntry {
                category: "custom".to_string(),
                command: "mytool".to_string(),
                mode: crate::config::ExternalShellMode::Snapshot,
                source_path: source.clone(),
                rendered_path: config.rendered_dir().join("shell/custom/tool.sh"),
                runtime: "bun".to_string(),
                bun_dependencies: None,
                dependency_hash: None,
                transforms: vec!["template".to_string()],
                env: vec!["OLD_KEY".to_string()],
                needs_source: true,
                content_hash: crate::install_core::hash_content(bytes),
            }],
            ..ShellManifest::default()
        }
        .save(&utils::runtime::RealHost, &config)
        .await
        .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_text, "update available");
        assert_eq!(
            row.changes,
            vec![
                UpdateChange::DeploymentChanged {
                    field: "mode",
                    from: "snapshot".to_string(),
                    to: "live".to_string(),
                },
                UpdateChange::DeploymentChanged {
                    field: "runtime",
                    from: "bun".to_string(),
                    to: "native".to_string(),
                },
                UpdateChange::DeploymentChanged {
                    field: "transforms",
                    from: "template".to_string(),
                    to: "none".to_string(),
                },
                UpdateChange::DeploymentChanged {
                    field: "env",
                    from: "OLD_KEY".to_string(),
                    to: "none".to_string(),
                },
                UpdateChange::DeploymentChanged {
                    field: "needs source",
                    from: "true".to_string(),
                    to: "false".to_string(),
                },
            ]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_lock_change_reports_update_available() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("tool.ts"), b"import 'zod'\n")
            .await
            .unwrap();
        fs::write(
            category.join("package.json"),
            b"{\"dependencies\":{\"zod\":\"4.0.0\"}}",
        )
        .await
        .unwrap();
        fs::write(category.join("bun.lock"), b"lockfileVersion = 1\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        fs::write(
            category.join("bun.lock"),
            b"lockfileVersion = 1\n# dependency changed\n",
        )
        .await
        .unwrap();
        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "custom/mytool")
            .unwrap();
        assert_eq!(row.status_text, "update available");
        assert!(row.changes.iter().any(|change| matches!(
            change,
            UpdateChange::DeploymentChanged {
                field: "dependency lock",
                ..
            }
        )));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_bun_source_change_reports_update_available() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        crate::shells::handle_install(&config, Some("agent"), false)
            .await
            .unwrap();

        let extracted = config.presets_dir().join("shell/agent/cc.ts");
        fs::write(&extracted, b"// stale extracted ccenv\n")
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "agent/ccenv")
            .expect("agent/ccenv row should exist");

        assert_eq!(row.status_sym, "↑");
        assert_eq!(row.status_text, "update available");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_shell_source_rename_reports_update_available() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/agent");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let old_source = if cfg!(windows) { "cc.ps1" } else { "cc.sh" };
        fs::write(
            cat_dir.join("shine.toml"),
            format!(
                "[[files]]\nsource = \"{old_source}\"\ntarget = \"ccenv\"\nneeds_source = true\n"
            ),
        )
        .await
        .unwrap();
        fs::write(cat_dir.join(old_source), b"# old sourced ccenv\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&config, Some("agent"), false)
            .await
            .unwrap();

        config.is_external_presets = false;
        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "agent/ccenv")
            .expect("embedded agent/ccenv row should exist");

        assert_eq!(row.status_sym, "↑");
        assert_eq!(row.status_text, "update available");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_shell_runtime_and_source_change_reports_update_available() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/agent");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let old_source = if cfg!(windows) { "cc.ps1" } else { "cc.sh" };
        fs::write(
            cat_dir.join("shine.toml"),
            format!(
                "[[files]]\nsource = \"{old_source}\"\ntarget = \"ccenv\"\nneeds_source = true\n"
            ),
        )
        .await
        .unwrap();
        fs::write(cat_dir.join(old_source), b"# old sourced ccenv\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        crate::shells::handle_install(&config, Some("agent"), false)
            .await
            .unwrap();

        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"cc.ts\"\ntarget = \"ccenv\"\nruntime = \"bun\"\nplatforms = [\"unix\", \"windows\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("cc.ts"), b"console.log('new ccenv');\n")
            .await
            .unwrap();

        let rows = build_shell_rows(&config).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.label == "agent/ccenv")
            .expect("external agent/ccenv row should exist");

        assert_eq!(row.status_sym, "↑");
        assert_eq!(row.status_text, "update available");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_env_change_reports_update_available() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        fs::write(
            cat_dir.join("set_proxy.sh"),
            b"#!/bin/bash\n# shine-template: true\nPROXY_NO_PROXY=\"@@PROXY_NO_PROXY@@\"\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        crate::shells::handle_install(&config, Some("proxy"), false)
            .await
            .unwrap();

        config.env.insert(
            "PROXY_NO_PROXY".to_string(),
            "localhost,127.0.0.1,::1,.local".to_string(),
        );

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
                    destination_root: None,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                },
                AppFile {
                    source_rel: PathBuf::from("themes/shine-light"),
                    target_rel: PathBuf::from("themes/shine-light"),
                    destination_root: None,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec!["template".to_string()],
                    install_strategy: AppInstallStrategy::Copy,
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                },
            ],
            list_mode: AppListMode::Category,
            post_upgrade: Vec::new(),
            post_install: Vec::new(),
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
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
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        let preset = config.presets_dir().join("app/sample");
        fs::create_dir_all(&preset).await.unwrap();
        fs::write(
            preset.join("shine.toml"),
            format!(
                "dest = {:?}\n[[files]]\nsource = \"config.toml\"\n[[files]]\nsource = \"theme.toml\"\n",
                dir.join(".config/sample").display().to_string()
            ),
        )
        .await
        .unwrap();
        fs::write(preset.join("config.toml"), b"config\n")
            .await
            .unwrap();
        fs::write(preset.join("theme.toml"), b"theme\n")
            .await
            .unwrap();

        let category = AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: Some(dir.join(".config/sample").display().to_string()),
            files: vec![
                AppFile {
                    source_rel: PathBuf::from("config.toml"),
                    target_rel: PathBuf::from("config.toml"),
                    destination_root: None,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                },
                AppFile {
                    source_rel: PathBuf::from("theme.toml"),
                    target_rel: PathBuf::from("theme.toml"),
                    destination_root: None,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                },
            ],
            list_mode: AppListMode::Files,
            post_upgrade: Vec::new(),
            post_install: Vec::new(),
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
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
        // SAFETY: env_lock() serialises all env-mutation tests in this module,
        // preventing concurrent writes to the process environment from other test threads.
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
        // SAFETY: env_lock() serialises all env-mutation tests in this module,
        // preventing concurrent writes to the process environment from other test threads.
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
