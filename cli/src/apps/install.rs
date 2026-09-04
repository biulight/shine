use super::metadata;
use super::report;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::presentation::{
    LifecycleReporter, PresentationEvent, TerminalInteraction, TerminalRenderer,
};
use anyhow::{Result, anyhow};
use shine_core::lifecycle::LifecycleOperation;
use shine_core::lifecycle::LifecycleResultV1;
#[cfg(test)]
use shine_core::lifecycle::LifecycleStatus;
use shine_core::runtime::{
    AppFileAction, AppLifecycleRequest, AppPlanRequest, PlanningInputVersions, RuntimeEvent,
    RuntimeObserver,
};
use std::collections::BTreeSet;

pub async fn handle_install(
    config: &Config,
    category: Option<&str>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    handle_install_approved(config, category, dry_run, force, true).await
}

pub async fn handle_install_approved(
    config: &Config,
    category: Option<&str>,
    dry_run: bool,
    force: bool,
    yes: bool,
) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_install_with_reporter(config, category, dry_run, force, yes, &mut renderer)
        .await
        .map(|_| ())
}

#[cfg(test)]
pub(crate) async fn handle_install_with_result(
    config: &Config,
    category: Option<&str>,
    dry_run: bool,
    force: bool,
) -> Result<LifecycleResultV1> {
    let mut renderer = TerminalRenderer::stdio();
    handle_install_with_reporter(config, category, dry_run, force, true, &mut renderer).await
}

async fn handle_install_with_reporter(
    config: &Config,
    category: Option<&str>,
    dry_run: bool,
    force: bool,
    yes: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    for line in crate::config::presets_note_lines(config) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    if dry_run {
        reporter.emit(PresentationEvent::stdout(report::dry_run_header_text()));
    }

    let plan_request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: category.map(str::to_string),
        force,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = if dry_run {
        None
    } else {
        crate::lifecycle_plan::review_plans(
            config,
            [crate::lifecycle_plan::LifecyclePlanRequest::app(
                plan_request.clone(),
                config,
            )],
            yes,
        )
        .await?
        .into_iter()
        .next()
    };
    let mut runtime = if let Some(reviewed) = &reviewed {
        crate::lifecycle_plan::prepare_runtime(config, reviewed).await?
    } else {
        crate::core_runtime::from_config(config).await?
    };
    let env = EnvConfig::load_or_init(config).await?;
    runtime.context_mut_for_cli().env = env.as_map().clone();
    let categories = runtime.app_categories(category)?;
    let total_available = categories.iter().map(|value| value.files.len()).sum();
    reporter.emit(PresentationEvent::stdout(report::app_configs_summary_text(
        total_available,
    )));
    let mut observer = InstallObserver {
        reporter,
        categories: &categories,
    };
    let mut interaction = TerminalInteraction;
    let core_report = if let Some(reviewed) = reviewed {
        match crate::lifecycle_plan::execute_reviewed(
            config,
            runtime,
            reviewed,
            shine_core::frontend::ExecutionOptions::default(),
            &mut observer,
            &mut interaction,
        )
        .await?
        {
            shine_core::frontend::OperationDetails::App(report) => *report,
            _ => unreachable!("reviewed operation result type"),
        }
    } else {
        runtime
            .preview_install_apps(
                AppLifecycleRequest {
                    target: category.map(str::to_string),
                    dry_run,
                    force,
                },
                &mut observer,
                &mut interaction,
            )
            .await?
    };
    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut backed_up = 0usize;
    let mut restart_hints = BTreeSet::new();
    for file in &core_report.files {
        let label = file.source.display().to_string();
        let display_name = format!("{}/{}", file.category, file.source.display());
        let transform_label = report::transform_label(&file.transforms);
        match file.action {
            AppFileAction::Installed | AppFileAction::BackedUp => {
                installed += 1;
                if file.action == AppFileAction::BackedUp {
                    let backup = file.backup.as_ref().expect("Core backed-up App report");
                    backed_up += 1;
                    observer.reporter.emit(PresentationEvent::stdout(
                        report::install_success_with_backup_text(
                            &label,
                            &transform_label,
                            &file.destination,
                            backup,
                            config,
                        ),
                    ));
                } else {
                    observer.reporter.emit(PresentationEvent::stdout(
                        report::install_success_text(
                            &label,
                            &transform_label,
                            &file.destination,
                            config,
                        ),
                    ));
                }
                if let Some(hint) = &file.restart_hint {
                    restart_hints.insert(hint.clone());
                }
            }
            AppFileAction::Unchanged => {
                skipped += 1;
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::already_managed_text(
                        &label,
                    )));
            }
            AppFileAction::PreviewInstall => {
                skipped += 1;
                observer
                    .reporter
                    .emit(PresentationEvent::stdout(report::dry_run_install_text(
                        &label,
                        &transform_label,
                        &file.destination,
                        config,
                    )));
            }
            AppFileAction::GeneratorPreserved => {
                skipped += 1;
                if let Some(error) = &file.generator_error {
                    observer.reporter.emit(PresentationEvent::stderr(
                        report::generator_unavailable_text(&display_name, &anyhow!(error.clone())),
                    ));
                }
            }
            AppFileAction::Failed => {
                if let Some(error) = &file.error {
                    observer
                        .reporter
                        .emit(PresentationEvent::stderr(report::install_error_text(
                            &display_name,
                            &anyhow!(error.clone()),
                        )));
                }
            }
            _ => skipped += 1,
        }
    }
    let summary_parts = report::install_summary_parts(installed, backed_up, skipped);
    observer.reporter.emit(PresentationEvent::BlankLine);
    observer
        .reporter
        .emit(PresentationEvent::stdout(report::done_summary_text(
            &summary_parts,
        )));
    for hint in restart_hints {
        observer
            .reporter
            .emit(PresentationEvent::stdout(report::restart_hint_text(&hint)));
    }
    let artifact_categories = categories
        .iter()
        .filter(|category| category.artifact.is_some())
        .map(|category| category.name.clone())
        .collect::<BTreeSet<_>>();
    let changed_categories = core_report
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.action,
                AppFileAction::Installed | AppFileAction::BackedUp
            )
        })
        .map(|file| file.category.clone())
        .collect();
    for category in report::artifact_apply_categories(&artifact_categories, changed_categories) {
        observer
            .reporter
            .emit(PresentationEvent::stdout(report::artifact_apply_hint_text(
                &category,
            )));
    }
    Ok(core_report.lifecycle)
}

struct InstallObserver<'a> {
    reporter: &'a mut dyn LifecycleReporter,
    categories: &'a [metadata::AppCategory],
}

impl RuntimeObserver for InstallObserver<'_> {
    fn emit(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Warning {
                code,
                target,
                detail,
            } => {
                let category = target
                    .as_deref()
                    .and_then(|value| value.strip_prefix("app/"))
                    .unwrap_or("app");
                if code == "app_hook_permission_required" {
                    let hooks = self
                        .categories
                        .iter()
                        .find(|value| value.name == category)
                        .map(|value| value.post_install.as_slice())
                        .unwrap_or_default();
                    let sequence = hooks
                        .iter()
                        .map(|hook| {
                            let program = match &hook.action {
                                shine_core::runtime::AppHookAction::Command(command) => {
                                    command.as_str()
                                }
                                shine_core::runtime::AppHookAction::Script { script, .. } => {
                                    script.to_str().unwrap_or("<script>")
                                }
                            };
                            std::iter::once(program)
                                .chain(hook.args.iter().map(String::as_str))
                                .map(crate::shell_quote::quote_if_needed)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect::<Vec<_>>()
                        .join(" && ");
                    self.reporter.emit(PresentationEvent::stdout(format!("  {} {category}: post-install hook skipped (run `shine trust grant app/{category}` after review; manual: {sequence})", report::symbol("!"))));
                } else {
                    self.reporter.emit(PresentationEvent::stderr(format!(
                        "  {} {category}: post-install hook failed: {detail}",
                        report::symbol("!")
                    )));
                }
            }
            RuntimeEvent::Progress {
                code: "app_hook_completed",
                target,
            } => {
                let category = target.strip_prefix("app/").unwrap_or(&target);
                self.reporter.emit(PresentationEvent::stdout(format!(
                    "  {} {category}: post-install hook completed",
                    report::symbol("✓")
                )));
            }
            RuntimeEvent::ProcessOutput { text, .. } => {
                for line in text.lines() {
                    self.reporter.emit(PresentationEvent::stdout(format!(
                        "     {}",
                        report::dim(line)
                    )));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    #[cfg(windows)]
    use super::super::uninstall::handle_uninstall;
    use super::super::uninstall::handle_uninstall_with_result;
    use super::*;
    use crate::apps::resolve_install_destination;
    use crate::config::Config;
    use crate::install_core::manifest::AppManifest;
    #[cfg(unix)]
    use crate::presets;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use shine_core::lifecycle::{LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1};
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-apps").await
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_then_uninstall_roundtrip() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;

        // Point HOME at the temp dir so ~ expands there
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let install_result = handle_install_with_result(&config, Some("git"), false, false)
            .await
            .unwrap();
        assert!(install_result.summary().changed > 0);
        assert!(
            install_result
                .outcomes
                .iter()
                .all(|outcome| outcome.target.starts_with("app/") && outcome.resource.is_some())
        );
        assert!(
            install_result
                .outcomes
                .iter()
                .filter(|outcome| outcome.status == LifecycleStatus::Failed)
                .all(|outcome| !outcome.diagnostic_codes.is_empty())
        );

        // At least the manifest should have entries
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            !manifest.entries.is_empty(),
            "manifest should have entries after install"
        );

        // Each installed file should exist
        for entry in &manifest.entries {
            assert!(
                entry.destination.exists(),
                "installed file should exist: {}",
                entry.destination.display()
            );
        }

        let no_op_result = handle_install_with_result(&config, Some("git"), false, false)
            .await
            .unwrap();
        assert!(no_op_result.summary().unchanged > 0);

        let uninstall_result =
            handle_uninstall_with_result(&config, Some("git"), false, false, false)
                .await
                .unwrap();
        assert!(uninstall_result.summary().changed > 0);
        assert!(uninstall_result.outcomes.iter().all(|outcome| {
            outcome.status != LifecycleStatus::Failed
                || outcome.resource.as_deref() == Some("artifact:teardown")
        }));

        let serialized = serde_json::to_string(&uninstall_result).unwrap();
        assert!(!serialized.contains(&dir.display().to_string()));

        let manifest_after = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest_after.entries.is_empty(),
            "manifest should be empty after uninstall"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn lifecycle_result_v1_json_shape_is_stable() {
        let mut result = LifecycleResultV1::new(LifecycleOperation::Install, false);
        result.push(LifecycleOutcomeV1::new(
            "app/sample",
            Some("config.toml"),
            LifecycleStatus::Changed,
            [
                LifecycleEffect::BackupCreated,
                LifecycleEffect::ResourceWritten,
                LifecycleEffect::ReceiptWritten,
            ],
        ));
        result.push(LifecycleOutcomeV1::new(
            "shell/sample/tool",
            Some("preset-cache"),
            LifecycleStatus::Pending,
            [
                LifecycleEffect::ReceiptWritePreviewed,
                LifecycleEffect::ReceiptRemovePreviewed,
                LifecycleEffect::CacheWritten,
                LifecycleEffect::CacheRemoved,
                LifecycleEffect::CachePurged,
                LifecycleEffect::CacheWritePreviewed,
                LifecycleEffect::CacheRemovePreviewed,
                LifecycleEffect::CodeExecuted,
                LifecycleEffect::CodeExecutionPreviewed,
            ],
        ));

        assert_eq!(
            serde_json::to_string_pretty(&result).unwrap(),
            r#"{
  "schema_version": 1,
  "operation": "install",
  "dry_run": false,
  "outcomes": [
    {
      "target": "app/sample",
      "resource": "config.toml",
      "status": "changed",
      "effects": [
        "backup-created",
        "resource-written",
        "receipt-written"
      ]
    },
    {
      "target": "shell/sample/tool",
      "resource": "preset-cache",
      "status": "pending",
      "effects": [
        "receipt-write-previewed",
        "receipt-remove-previewed",
        "cache-written",
        "cache-removed",
        "cache-purged",
        "cache-write-previewed",
        "cache-remove-previewed",
        "code-executed",
        "code-execution-previewed"
      ]
    }
  ]
}"#
        );
    }

    #[tokio::test]
    async fn structured_roundtrip_records_backup_creation_and_restore() {
        let dir = make_temp_dir().await;
        let category_dir = dir.join("presets/app/sample");
        let destination_root = dir.join("destination");
        fs::create_dir_all(&category_dir).await.unwrap();
        fs::create_dir_all(&destination_root).await.unwrap();
        fs::write(
            category_dir.join("shine.toml"),
            format!(
                "description = \"Sample\"\ndest = {:?}\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"config.toml\"\n",
                destination_root.to_string_lossy()
            ),
        )
        .await
        .unwrap();
        fs::write(category_dir.join("config.toml"), b"managed\n")
            .await
            .unwrap();
        let destination = destination_root.join("config.toml");
        fs::write(&destination, b"original\n").await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let install = handle_install_with_result(&config, Some("sample"), false, false)
            .await
            .unwrap();
        assert_eq!(install.summary().changed, 1);
        assert!(install.outcomes.iter().any(|outcome| {
            outcome.resource.as_deref() == Some("config.toml")
                && outcome.effects.contains(&LifecycleEffect::BackupCreated)
        }));

        let uninstall = handle_uninstall_with_result(&config, Some("sample"), false, false, false)
            .await
            .unwrap();
        assert_eq!(uninstall.summary().changed, 1);
        assert!(
            uninstall.outcomes[0]
                .effects
                .contains(&LifecycleEffect::BackupRestored)
        );
        assert_eq!(fs::read(&destination).await.unwrap(), b"original\n");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn future_app_manifest_fails_before_destination_mutation() {
        let dir = make_temp_dir().await;
        let category_dir = dir.join("presets/app/sample");
        let destination_root = dir.join("destination");
        fs::create_dir_all(&category_dir).await.unwrap();
        fs::write(
            category_dir.join("shine.toml"),
            format!(
                "description = \"Sample\"\ndest = {:?}\n\n[[files]]\nsource = \"config.toml\"\n",
                destination_root.to_string_lossy()
            ),
        )
        .await
        .unwrap();
        fs::write(category_dir.join("config.toml"), b"managed\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        fs::write(
            config.shine_dir().join("app-manifest.toml"),
            "schema_version = 2\n",
        )
        .await
        .unwrap();

        let error = handle_install_with_result(&config, Some("sample"), false, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("newer than this Shine supports"));
        assert!(!destination_root.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_install_dry_run_previews_cache_without_extracting_it() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        let result = handle_install_with_result(&config, Some("git"), true, false)
            .await
            .unwrap();

        let cache = result
            .outcomes
            .iter()
            .find(|outcome| outcome.resource.as_deref() == Some("preset-cache"))
            .unwrap();
        assert_eq!(cache.status, LifecycleStatus::Previewed);
        assert_eq!(cache.effects, [LifecycleEffect::CacheWritePreviewed]);
        assert!(!config.presets_dir().join("app/git").exists());
        assert!(!config.shine_dir().join("app-manifest.toml").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn future_app_manifest_rejects_embedded_cache_extraction() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::write(
            config.shine_dir().join("app-manifest.toml"),
            "schema_version = 2\n",
        )
        .await
        .unwrap();

        let error = handle_install_with_result(&config, Some("git"), false, false)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("newer than this Shine supports"));
        assert!(!config.presets_dir().join("app/git").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_is_idempotent() {
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
        let manifest_first = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let count_first = manifest_first.entries.len();

        handle_install(&config, Some("git"), false, false)
            .await
            .unwrap();
        let manifest_second = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();

        assert_eq!(
            manifest_second.entries.len(),
            count_first,
            "re-install must not duplicate manifest entries"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn post_install_hook_runs_only_when_a_file_changes() {
        let dir = make_temp_dir().await;
        let dest_root = dir.join("dest").to_string_lossy().replace('\\', "/");
        let marker = dir.join("post-install-ran");
        let category_dir = dir.join("presets/app/hooktest");
        fs::create_dir_all(&category_dir).await.unwrap();
        fs::write(
            category_dir.join("shine.toml"),
            format!(
                "description = \"hook test\"\n\
dest = \"{dest_root}\"\n\
post_install = {{ command = \"/bin/sh\", args = [\"-c\", \"touch {marker}\"] }}\n\n\
[permissions]\n\
schema_version = 1\n\
commands = [\"/bin/sh\"]\n\n\
[[files]]\n\
source = \"file.conf\"\n",
                marker = marker.display()
            ),
        )
        .await
        .unwrap();
        fs::write(category_dir.join("file.conf"), b"hello\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        crate::trust::grant_current_for_test(&config, "app/hooktest").await;

        // First install writes the file → post_install fires.
        handle_install(&config, Some("hooktest"), false, false)
            .await
            .unwrap();
        assert!(marker.exists(), "post_install must run on first install");

        // Second install changes nothing → hook must not fire again.
        fs::remove_file(&marker).await.unwrap();
        handle_install(&config, Some("hooktest"), false, false)
            .await
            .unwrap();
        assert!(
            !marker.exists(),
            "post_install must not run when no file changed"
        );

        // Replacement install (force) rewrites the file → post_install fires again.
        handle_install(&config, Some("hooktest"), false, true)
            .await
            .unwrap();
        assert!(
            marker.exists(),
            "post_install must run on replacement install"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_dry_run_uses_generator_fallback_without_executing_code() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir().await;
        let destination = dir.join("destination");
        let marker = dir.join("generator-ran");
        let category = dir.join("presets/app/generated");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            format!(
                "dest = {:?}\n[[files]]\nsource = \"fallback.txt\"\ngenerator = {{ script = \"generate.sh\", env = [\"RUN\"], when_env = \"RUN\" }}\n",
                destination.to_string_lossy()
            ),
        )
        .await
        .unwrap();
        fs::write(category.join("fallback.txt"), b"fallback\n")
            .await
            .unwrap();
        let generator = category.join("generate.sh");
        fs::write(
            &generator,
            format!("#!/bin/sh\ntouch {:?}\necho generated\n", marker),
        )
        .await
        .unwrap();
        let mut permissions = fs::metadata(&generator).await.unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&generator, permissions).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.env.insert("RUN".to_string(), "yes".to_string());
        let result = handle_install_with_result(&config, Some("generated"), true, false)
            .await
            .unwrap();

        assert!(result.dry_run);
        assert_eq!(result.summary().previewed, 1);
        assert!(result.outcomes.iter().any(|outcome| {
            outcome.effects
                == vec![
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ]
        }));
        assert!(!marker.exists());
        assert!(!destination.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn install_missing_category_errors() {
        let dir = std::env::temp_dir().join("shine-apps-missing-category");
        let config = Config::new_for_test(&dir);

        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(handle_install(&config, Some("docker"), true, false))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("app preset category not found: docker")
        );
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn docker_desktop_install_and_uninstall_only_manage_proxy_keys() {
        let dir = make_temp_dir().await;
        let dest_root = dir
            .join("desktop-settings")
            .to_string_lossy()
            .replace('\\', "/");
        let category_dir = dir.join("presets/app/docker-desktop-test");
        fs::create_dir_all(&category_dir).await.unwrap();
        fs::write(
            category_dir.join("shine.toml"),
            format!(
                "description = \"Docker Desktop proxy settings\"\n\
dest = \"{dest_root}\"\n\n\
[permissions]\n\
schema_version = 1\n\n\
[[files]]\n\
source = \"settings-store.jsonc\"\n\
target = \"settings-store.json\"\n\
transforms = [\"template\", \"jsonc-to-json\"]\n\
install_mode = \"json-merge\"\n\
managed_keys = [\"proxy\", \"containersProxy\"]\n"
            ),
        )
        .await
        .unwrap();
        fs::write(
            category_dir.join("settings-store.jsonc"),
            br#"{
  "proxy": {
    "mode": "manual",
    "http": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@",
    "https": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@"
  },
  "containersProxy": {
    "mode": "manual",
    "http": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@",
    "https": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@"
  }
}"#,
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let destination = dir.join("desktop-settings").join("settings-store.json");
        fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(
            &destination,
            br#"{
  "theme": "dark",
  "analyticsEnabled": true
}"#,
        )
        .await
        .unwrap();

        handle_install(&config, Some("docker-desktop-test"), false, false)
            .await
            .unwrap();

        let mut installed: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).await.unwrap()).unwrap();
        assert_eq!(installed["theme"], serde_json::json!("dark"));
        assert_eq!(installed["analyticsEnabled"], serde_json::json!(true));
        assert_eq!(installed["proxy"]["mode"], serde_json::json!("manual"));
        assert_eq!(
            installed["containersProxy"]["mode"],
            serde_json::json!("manual")
        );

        installed["theme"] = serde_json::json!("light");
        fs::write(&destination, serde_json::to_vec_pretty(&installed).unwrap())
            .await
            .unwrap();

        handle_uninstall(&config, Some("docker-desktop-test"), false, false, false)
            .await
            .unwrap();

        let removed: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).await.unwrap()).unwrap();
        assert_eq!(
            removed,
            serde_json::json!({
                "analyticsEnabled": true,
                "theme": "light"
            })
        );

        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.entries.is_empty(),
            "docker-desktop uninstall should clear manifest entries"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_vim_under_directory_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        presets::extract_prefix("app/vim", config.presets_dir(), false)
            .await
            .unwrap();

        let categories = metadata::load_installed_categories(&config, Some("vim"))
            .await
            .unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        let vimrc = vim
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("vimrc"))
            .unwrap();
        let destination = resolve_install_destination(vim, vimrc, &config).unwrap();
        assert_eq!(destination, dir.join(".vim").join("vimrc"));

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_ghostty_config_under_config_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        presets::extract_prefix("app/ghostty", config.presets_dir(), false)
            .await
            .unwrap();

        let categories = metadata::load_installed_categories(&config, Some("ghostty"))
            .await
            .unwrap();
        let ghostty = categories.iter().find(|c| c.name == "ghostty").unwrap();
        let config_file = ghostty
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("config.ghostty"))
            .unwrap();
        let destination = resolve_install_destination(ghostty, config_file, &config).unwrap();
        assert_eq!(
            destination,
            dir.join(".config/ghostty").join("config.ghostty")
        );

        let light_theme = ghostty
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("themes/iTerm2 Solarized Light"))
            .unwrap();
        let light_destination = resolve_install_destination(ghostty, light_theme, &config).unwrap();
        assert_eq!(
            light_destination,
            dir.join(".config/ghostty")
                .join("themes/light_iTerm2 Solarized Light")
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_renders_ghostty_light_and_dark_background_images() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let mut config = Config::new_for_test(&dir);
        config.env.insert(
            "GHOSTTY_BG_LIGHT".into(),
            "/tmp/shine-light-wallpaper.png".into(),
        );
        config.env.insert(
            "GHOSTTY_BG_DARK".into(),
            "/tmp/shine-dark-wallpaper.png".into(),
        );
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("ghostty"), false, false)
            .await
            .unwrap();

        let config_text = fs::read_to_string(dir.join(".config/ghostty/config.ghostty"))
            .await
            .unwrap();
        assert!(config_text.contains("theme = light:Shine Light,dark:dark_Alien Blood"));

        let default_light_theme =
            fs::read_to_string(dir.join(".config/ghostty/themes/Shine Light"))
                .await
                .unwrap();
        assert!(default_light_theme.contains("background-image = /tmp/shine-light-wallpaper.png"));

        let light_theme =
            fs::read_to_string(dir.join(".config/ghostty/themes/light_Github Light Default"))
                .await
                .unwrap();
        assert!(light_theme.contains("background = #ffffff"));
        assert!(light_theme.contains("palette = 4=#0969da"));
        assert!(light_theme.contains("cursor-color = #0969da"));
        assert!(light_theme.contains("background-image = /tmp/shine-light-wallpaper.png"));

        let dark_theme = fs::read_to_string(dir.join(".config/ghostty/themes/dark_Alien Blood"))
            .await
            .unwrap();
        assert!(dark_theme.contains("background = #0f1610"));
        assert!(dark_theme.contains("palette = 10=#18e000"));
        assert!(dark_theme.contains("cursor-color = #73fa91"));
        assert!(dark_theme.contains("background-image = /tmp/shine-dark-wallpaper.png"));

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
