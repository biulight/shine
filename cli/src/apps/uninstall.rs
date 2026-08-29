use super::report;
use crate::config::Config;
#[cfg(test)]
use crate::install_core::manifest::{AppEntry, AppManifest};
use crate::presentation::{LifecycleReporter, PresentationEvent, TerminalRenderer};
use anyhow::Result;
#[cfg(test)]
use shine_core::lifecycle::LifecycleEffect;
use shine_core::lifecycle::LifecycleOperation;
use shine_core::lifecycle::LifecycleResultV1;
use shine_core::runtime::{AppPlanRequest, PlanningInputVersions};

pub async fn handle_uninstall(
    config: &Config,
    category: Option<&str>,
    force: bool,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    handle_uninstall_approved(config, category, force, purge, dry_run, true).await
}

pub async fn handle_uninstall_approved(
    config: &Config,
    category: Option<&str>,
    force: bool,
    purge: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, category, force, purge, dry_run, yes, &mut renderer)
        .await
        .map(|_| ())
}

#[cfg(test)]
pub(crate) async fn handle_uninstall_with_result(
    config: &Config,
    category: Option<&str>,
    force: bool,
    purge: bool,
    dry_run: bool,
) -> Result<LifecycleResultV1> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, category, force, purge, dry_run, true, &mut renderer)
        .await
}

async fn handle_uninstall_with_reporter(
    config: &Config,
    category: Option<&str>,
    force: bool,
    purge: bool,
    dry_run: bool,
    yes: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    if dry_run {
        reporter.emit(PresentationEvent::stdout(report::dry_run_header_text()));
    }
    let plan_request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: category.map(str::to_string),
        force,
        purge,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = if dry_run {
        None
    } else {
        crate::lifecycle_plan::review_plans(
            config,
            [crate::lifecycle_plan::LifecyclePlanRequest::app(
                plan_request,
                config,
            )],
            yes,
        )
        .await?
        .into_iter()
        .next()
    };
    let runtime = if let Some(reviewed) = &reviewed {
        crate::lifecycle_plan::prepare_runtime(config, reviewed).await?
    } else {
        crate::core_runtime::from_config(config).await?
    };
    let mut observer = UninstallObserver { reporter };
    let mut interaction = crate::presentation::TerminalInteraction;
    let core_report = if let Some(reviewed) = &reviewed {
        runtime
            .uninstall_apps_approved(
                match &reviewed.request {
                    crate::lifecycle_plan::LifecyclePlanRequest::App(request) => request.clone(),
                    _ => unreachable!("reviewed App Plan"),
                },
                &reviewed.approval,
                &mut observer,
                &mut interaction,
            )
            .await?
    } else {
        runtime
            .preview_uninstall_apps(
                shine_core::runtime::AppUninstallLifecycleRequest {
                    target: category.map(str::to_string),
                    dry_run,
                    force,
                    purge,
                },
                &mut observer,
                &mut interaction,
            )
            .await?
    };
    if let Some(category) = category.filter(|_| core_report.files.is_empty()) {
        observer
            .reporter
            .emit(PresentationEvent::stdout(report::no_installed_files_text(
                category,
            )));
        return Ok(core_report.lifecycle);
    }
    let mut removed = 0usize;
    let mut restored = 0usize;
    let mut user_modified = 0usize;
    let mut skipped = 0usize;
    for file in &core_report.files {
        match file.action {
            shine_core::runtime::AppFileAction::Removed => {
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::removed_text(
                        config,
                        &file.destination,
                    )));
                removed += 1;
            }
            shine_core::runtime::AppFileAction::Restored => {
                let backup = file
                    .backup
                    .as_ref()
                    .expect("Core restored App backup report");
                observer.reporter.emit(PresentationEvent::stdout(
                    report::removed_with_restore_text(config, &file.destination, backup),
                ));
                removed += 1;
                restored += 1;
            }
            shine_core::runtime::AppFileAction::ForceRemoved => {
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::force_removed_text(
                        &file.destination,
                    )));
                removed += 1;
            }
            shine_core::runtime::AppFileAction::ForceRestored => {
                let backup = file
                    .backup
                    .as_ref()
                    .expect("Core force-restored App backup report");
                observer.reporter.emit(PresentationEvent::stdout(
                    report::force_removed_with_restore_text(&file.destination, backup),
                ));
                removed += 1;
                restored += 1;
            }
            shine_core::runtime::AppFileAction::Missing => {
                observer.reporter.emit(PresentationEvent::stdout(
                    report::uninstall_not_found_text(config, &file.destination),
                ));
                skipped += 1;
            }
            shine_core::runtime::AppFileAction::UserModified => {
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::user_modified_kept_text(
                        config,
                        &file.destination,
                    )));
                user_modified += 1;
            }
            shine_core::runtime::AppFileAction::PreviewRemove => {
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::uninstall_dry_run_text(
                        config,
                        &file.destination,
                    )));
                skipped += 1;
            }
            shine_core::runtime::AppFileAction::Failed => {
                let error = anyhow::anyhow!(
                    file.error
                        .clone()
                        .unwrap_or_else(|| "App uninstall failed".to_string())
                );
                observer
                    .reporter
                    .emit(PresentationEvent::stderr(report::uninstall_error_text(
                        config,
                        &file.destination,
                        &error,
                    )));
            }
            _ => {}
        }
    }
    if purge && !config.is_external_presets {
        observer
            .reporter
            .emit(PresentationEvent::stdout(match category {
                Some(category) => report::purge_category_text(category),
                None => report::purge_all_text(),
            }));
    }
    let summary_parts = report::uninstall_summary_parts(removed, restored, user_modified, skipped);
    observer.reporter.emit(PresentationEvent::BlankLine);
    observer
        .reporter
        .emit(PresentationEvent::stdout(report::done_summary_text(
            &summary_parts,
        )));
    Ok(core_report.lifecycle)
}

struct UninstallObserver<'a> {
    reporter: &'a mut dyn LifecycleReporter,
}

impl shine_core::runtime::RuntimeObserver for UninstallObserver<'_> {
    fn emit(&mut self, event: shine_core::runtime::RuntimeEvent) {
        if let shine_core::runtime::RuntimeEvent::Warning {
            code,
            target,
            detail,
        } = event
        {
            let category = target
                .as_deref()
                .and_then(|value| value.strip_prefix("app/"))
                .unwrap_or("app");
            if code == "app_artifact_permission_required" {
                self.reporter.emit(PresentationEvent::stdout(format!("  {} {category}: artifact teardown skipped (set allow_app_hooks = true to allow external app hooks; manual: shine app artifact remove {category})", report::symbol("!"))));
            } else {
                self.reporter.emit(PresentationEvent::stderr(format!(
                    "  {} {category}: artifact teardown failed: {detail}",
                    report::symbol("!")
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    #[cfg(unix)]
    use super::super::install::handle_install;
    use super::*;
    use crate::install_core::manifest::AppInstallStrategy;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-apps").await
    }

    #[cfg(unix)]
    async fn write_external_sample_app(dir: &std::path::Path, body: &[u8]) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let manifest = "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n".to_string();
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("git"), false, false)
            .await
            .unwrap();

        let manifest_before = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let count_before = manifest_before.entries.len();

        let result = handle_uninstall_with_result(&config, Some("git"), false, false, true)
            .await
            .unwrap();
        assert!(result.dry_run);
        assert_eq!(
            result
                .outcomes
                .iter()
                .filter(|outcome| {
                    outcome.effects
                        == vec![
                            LifecycleEffect::ResourceRemovePreviewed,
                            LifecycleEffect::ReceiptRemovePreviewed,
                        ]
                })
                .count(),
            count_before
        );
        assert!(
            result
                .outcomes
                .iter()
                .filter(|outcome| {
                    outcome.resource.as_deref() != Some("artifact:teardown")
                        && outcome.resource.as_deref() != Some("preset-cache")
                })
                .all(|outcome| {
                    outcome.effects
                        == vec![
                            LifecycleEffect::ResourceRemovePreviewed,
                            LifecycleEffect::ReceiptRemovePreviewed,
                        ]
                })
        );

        let manifest_after = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert_eq!(
            manifest_after.entries.len(),
            count_before,
            "dry-run must not modify manifest"
        );
        for entry in &manifest_before.entries {
            assert!(
                entry.destination.exists(),
                "dry-run must not remove installed files"
            );
        }

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_force_removes_user_modified_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"debug\": true\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"debug\": false}\n").await.unwrap();

        let result = handle_uninstall_with_result(&config, Some("sample"), true, false, false)
            .await
            .unwrap();
        assert_eq!(result.summary().changed, 1);
        assert!(
            result.outcomes[0]
                .effects
                .contains(&LifecycleEffect::UserModificationOverridden)
        );

        let manifest_after = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest_after.entries.is_empty(),
            "force uninstall should remove manifest entry"
        );
        assert!(
            !dest.exists(),
            "force uninstall should remove modified file"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_specific_category_only_removes_that_category() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        // Install two categories so targeted removal can prove isolation.
        handle_install(&config, Some("git"), false, false)
            .await
            .unwrap();
        handle_install(&config, Some("starship"), false, false)
            .await
            .unwrap();
        let manifest_all = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let total = manifest_all.entries.len();
        assert!(total > 0, "need at least one installed entry");

        // Find a category that was installed
        let first_category = manifest_all
            .entries
            .iter()
            .find_map(|e| {
                e.source
                    .strip_prefix("app/")
                    .and_then(|s| s.split('/').next())
                    .map(|s| s.to_string())
            })
            .expect("no category found in manifest");

        let category_count = manifest_all
            .entries
            .iter()
            .filter(|e| e.source.starts_with(&format!("app/{first_category}/")))
            .count();

        // Uninstall only that category
        let result =
            handle_uninstall_with_result(&config, Some(&first_category), false, false, false)
                .await
                .unwrap();
        assert!(
            result
                .outcomes
                .iter()
                .all(|outcome| outcome.target == format!("app/{first_category}"))
        );

        let manifest_after = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert_eq!(
            manifest_after.entries.len(),
            total - category_count,
            "only entries for '{first_category}' should be removed"
        );
        // No remaining entry belongs to the uninstalled category
        let prefix = format!("app/{first_category}/");
        assert!(
            manifest_after
                .entries
                .iter()
                .all(|e| !e.source.starts_with(&prefix)),
            "uninstalled category must not appear in manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn structured_uninstall_preserves_user_modified_resource_and_receipt() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        let destination = dir.join("destination/config.toml");
        fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(&destination, b"user change\n").await.unwrap();
        let manifest = AppManifest {
            entries: vec![AppEntry {
                source: "app/sample/config.toml".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install_core::hash_content(b"installed\n"),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..AppManifest::default()
        };
        manifest
            .save(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();

        let result = handle_uninstall_with_result(&config, None, false, false, false)
            .await
            .unwrap();

        assert_eq!(result.summary().preserved, 1);
        assert_eq!(
            result.outcomes[0].effects,
            vec![LifecycleEffect::UserResourcePreserved]
        );
        assert_eq!(fs::read(&destination).await.unwrap(), b"user change\n");
        assert_eq!(
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn structured_uninstall_reports_stale_receipt_cleanup_as_change() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        let manifest = AppManifest {
            entries: vec![AppEntry {
                source: "app/sample/missing.toml".to_string(),
                destination: dir.join("destination/missing.toml"),
                backup: None,
                content_hash: crate::install_core::hash_content(b"installed\n"),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..AppManifest::default()
        };
        manifest
            .save(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();

        let result = handle_uninstall_with_result(&config, None, false, false, false)
            .await
            .unwrap();

        assert_eq!(result.summary().changed, 1);
        assert_eq!(
            result.outcomes[0].effects,
            vec![LifecycleEffect::ReceiptRemoved]
        );
        assert!(
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_category_and_global_purge_record_cache_and_manifest_effects() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let category_cache = config.presets_dir().join("app/git");
        fs::create_dir_all(&category_cache).await.unwrap();
        fs::write(category_cache.join("orphan"), b"cache")
            .await
            .unwrap();
        let manifest = AppManifest {
            entries: vec![AppEntry {
                source: "app/git/gitconfig".to_string(),
                destination: dir.join("missing-gitconfig"),
                backup: None,
                content_hash: 1,
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..AppManifest::default()
        };
        manifest
            .save(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();

        let category = handle_uninstall_with_result(&config, Some("git"), false, true, false)
            .await
            .unwrap();
        let category_purge = category
            .outcomes
            .iter()
            .find(|outcome| outcome.resource.as_deref() == Some("purge"))
            .unwrap();
        assert_eq!(category_purge.target, "app/git");
        assert!(
            category_purge
                .effects
                .contains(&LifecycleEffect::CachePurged)
        );

        let global_cache = config.presets_dir().join("app/other");
        fs::create_dir_all(&global_cache).await.unwrap();
        fs::write(global_cache.join("orphan"), b"cache")
            .await
            .unwrap();
        let global = handle_uninstall_with_result(&config, None, false, true, false)
            .await
            .unwrap();
        let global_purge = global
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "app" && outcome.resource.as_deref() == Some("purge"))
            .unwrap();
        assert!(global_purge.effects.contains(&LifecycleEffect::CachePurged));
        assert!(
            global_purge
                .effects
                .contains(&LifecycleEffect::ReceiptRemoved)
        );
        assert!(!config.shine_dir().join("app-manifest.toml").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_unknown_category_returns_early() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        // Nothing installed — uninstalling a specific category should succeed silently
        handle_uninstall(&config, Some("nonexistent"), false, false, false)
            .await
            .unwrap();

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
