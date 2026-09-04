//! Explicit refresh of manifest-owned generated app files.

use anyhow::{Result, bail};
use std::path::Path;

use crate::colors;
use crate::config::Config;
use crate::presentation::TerminalInteraction;
use shine_core::runtime::{
    AppFileAction, AppRefreshPlanRequest, PlanningInputVersions, RuntimeEvent, RuntimeObserver,
};

use super::report::{print_install_error, print_install_success};

pub async fn handle_refresh(
    config: &Config,
    category: &str,
    file_selector: Option<&str>,
    force: bool,
) -> Result<()> {
    handle_refresh_approved(config, category, file_selector, force, true).await
}

pub async fn handle_refresh_approved(
    config: &Config,
    category: &str,
    file_selector: Option<&str>,
    force: bool,
    yes: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let plan_request = AppRefreshPlanRequest {
        category: category.to_string(),
        file: file_selector.map(Path::new).map(Path::to_path_buf),
        force,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::app_refresh(
            plan_request.clone(),
            config,
        )],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed App refresh Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let plan_request = reviewed_app_refresh_request(&reviewed.request);

    println!(
        "{}",
        colors::bold(&format!("Refreshing app generators: {category}"))
    );
    let mut observer = RefreshObserver;
    let mut interaction = TerminalInteraction;
    let report = runtime
        .refresh_app_generators_approved(
            plan_request,
            &reviewed.approval,
            &mut observer,
            &mut interaction,
        )
        .await?;
    let single_label = report
        .files
        .first()
        .filter(|_| report.files.len() == 1)
        .map(|file| format!("{category}/{}", file.source.display()));
    let mut updated = 0;
    let mut unchanged = 0;
    let mut failed = 0;
    for file in report.files {
        let label = format!("{category}/{}", file.source.display());
        match file.action {
            AppFileAction::Installed | AppFileAction::BackedUp => {
                print_install_success(&label, "", &file.destination, config);
                updated += 1;
            }
            AppFileAction::Unchanged => {
                println!(
                    "  {} {label}  {}",
                    colors::dim("-"),
                    colors::dim("already up to date")
                );
                unchanged += 1;
            }
            AppFileAction::UserModified => {
                eprintln!(
                    "  {} {label}: user-modified, kept (use --force to overwrite)",
                    colors::symbol("!")
                );
                failed += 1;
            }
            AppFileAction::Failed => {
                print_install_error(&label, &anyhow::anyhow!(file.error.unwrap_or_default()));
                failed += 1;
            }
            _ => unchanged += 1,
        }
    }

    println!(
        "{}",
        refresh_summary_text(single_label.as_deref(), updated, unchanged, failed)
    );
    if failed > 0 {
        bail!("{failed} generated app file(s) failed to refresh");
    }
    Ok(())
}

fn reviewed_app_refresh_request(
    request: &crate::lifecycle_plan::LifecyclePlanRequest,
) -> AppRefreshPlanRequest {
    match request {
        crate::lifecycle_plan::LifecyclePlanRequest::AppRefresh(request) => request.clone(),
        _ => unreachable!("reviewed App refresh Plan must retain its refresh request"),
    }
}

fn refresh_summary_text(
    single_label: Option<&str>,
    updated: usize,
    unchanged: usize,
    failed: usize,
) -> String {
    let total = updated + unchanged + failed;
    if total == 1
        && let Some(label) = single_label
    {
        if failed == 1 {
            return format!(
                "{} {}",
                colors::symbol("!"),
                colors::yellow(&format!("Refresh incomplete: {label} failed"))
            );
        }
        if updated == 1 {
            return format!(
                "{} {}",
                colors::symbol("✓"),
                colors::green(&format!("Refresh complete: {label} updated"))
            );
        }
        return format!(
            "{} {}",
            colors::symbol("✓"),
            colors::green(&format!("Already up to date: {label}"))
        );
    }

    let mut parts = Vec::new();
    crate::output::push_count(&mut parts, updated, colors::green, "updated");
    crate::output::push_count(&mut parts, unchanged, colors::dim, "unchanged");
    crate::output::push_count(&mut parts, failed, colors::yellow, "failed");
    let detail = if parts.is_empty() {
        colors::dim("nothing changed")
    } else {
        parts.join(&colors::dim(", "))
    };
    let (symbol, conclusion) = if failed > 0 {
        ("!", "Refresh incomplete")
    } else {
        ("✓", "Refresh complete")
    };
    format!("{} {conclusion}: {detail}", colors::symbol(symbol))
}

struct RefreshObserver;

impl RuntimeObserver for RefreshObserver {
    fn emit(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Warning { detail, .. } => eprintln!("  {} {detail}", colors::symbol("!")),
            RuntimeEvent::ProcessOutput { text, .. } => {
                for line in text.lines() {
                    println!("     {}", colors::dim(line));
                }
            }
            RuntimeEvent::Progress {
                code: "app_hook_completed",
                target,
            } => {
                println!(
                    "  {} {}: post-upgrade hook completed",
                    colors::symbol("✓"),
                    target.trim_start_matches("app/")
                );
            }
            _ => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::apps::metadata;
    use crate::apps::{handle_install, handle_upgrade_installed};
    use crate::install_core::manifest::AppManifest;
    use crate::status::{FileStatus, app_entry_status};
    use shine_core::runtime::OpaqueSecretVersion;
    use std::os::unix::fs::PermissionsExt;
    use tokio::fs;

    async fn write_fixture(root: &Path, two_files: bool) -> Config {
        let mut config = Config::new_for_test(root);
        config.is_external_presets = true;
        config
            .env
            .insert("SOURCE_URL".to_string(), "https://example.test".to_string());
        let app_dir = config.presets_dir().join("app/sample");
        fs::create_dir_all(&app_dir).await.unwrap();
        let second = if two_files {
            r#"

[[files]]
source = "second.txt"
generator = { script = "second.sh", env = ["SOURCE_URL"], when_env = "SOURCE_URL", auto = false }
"#
        } else {
            ""
        };
        fs::write(
            app_dir.join("shine.toml"),
            format!(
                r#"description = "sample"
dest = "{}"

[permissions]
schema_version = 1
filesystem = [
  {{ access = ["execute"], base = "preset", path = "first.sh" }},
  {{ access = ["execute"], base = "preset", path = "second.sh" }},
]
environment = [{{ name = "SOURCE_URL", sensitivity = "secret" }}]

[[files]]
source = "first.txt"
generator = {{ script = "first.sh", env = ["SOURCE_URL"], when_env = "SOURCE_URL", auto = false }}
{second}"#,
                root.join("dest").display()
            ),
        )
        .await
        .unwrap();
        fs::write(app_dir.join("first.txt"), b"fallback-first\n")
            .await
            .unwrap();
        fs::write(app_dir.join("first.payload"), b"first-v1\n")
            .await
            .unwrap();
        write_generator(&app_dir.join("first.sh"), "first").await;
        if two_files {
            fs::write(app_dir.join("second.txt"), b"fallback-second\n")
                .await
                .unwrap();
            fs::write(app_dir.join("second.payload"), b"second-v1\n")
                .await
                .unwrap();
            write_generator(&app_dir.join("second.sh"), "second").await;
        }
        crate::trust::grant_current_for_test(&config, "app/sample").await;
        config
    }

    async fn write_generator(path: &Path, stem: &str) {
        fs::write(
            path,
            format!(
                "#!/bin/sh\nprintf x >> '{counter}'\ncat '{payload}'\n",
                counter = path
                    .parent()
                    .unwrap()
                    .join(format!("{stem}.runs"))
                    .display(),
                payload = path
                    .parent()
                    .unwrap()
                    .join(format!("{stem}.payload"))
                    .display()
            ),
        )
        .await
        .unwrap();
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
    }

    #[test]
    fn refresh_execution_reuses_the_reviewed_input_versions() {
        let mut input_versions = PlanningInputVersions::default();
        input_versions
            .insert_secret_version("SOURCE_URL", OpaqueSecretVersion::new("test-version"));
        let request =
            crate::lifecycle_plan::LifecyclePlanRequest::AppRefresh(AppRefreshPlanRequest {
                category: "sample".to_string(),
                file: Some(Path::new("first.txt").to_path_buf()),
                force: false,
                input_versions: input_versions.clone(),
            });

        assert_eq!(
            reviewed_app_refresh_request(&request).input_versions,
            input_versions
        );
    }

    #[test]
    fn refresh_summary_names_single_files_and_distinguishes_failures() {
        assert_eq!(
            refresh_summary_text(Some("sample/first.txt"), 1, 0, 0),
            "✓ Refresh complete: sample/first.txt updated"
        );
        assert_eq!(
            refresh_summary_text(Some("sample/first.txt"), 0, 1, 0),
            "✓ Already up to date: sample/first.txt"
        );
        assert_eq!(
            refresh_summary_text(Some("sample/first.txt"), 0, 0, 1),
            "! Refresh incomplete: sample/first.txt failed"
        );
        assert_eq!(
            refresh_summary_text(None, 2, 1, 0),
            "✓ Refresh complete: 2 updated, 1 unchanged"
        );
        assert_eq!(
            refresh_summary_text(None, 1, 0, 1),
            "! Refresh incomplete: 1 updated, 1 failed"
        );
    }

    #[tokio::test]
    async fn manual_generator_skips_status_and_upgrade_but_refreshes_explicitly() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, false).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();

        let app_dir = config.presets_dir().join("app/sample");
        let dest = root.join("dest/first.txt");
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "x"
        );
        fs::write(app_dir.join("first.payload"), b"first-v2\n")
            .await
            .unwrap();

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let cat = &categories[0];
        let file = &cat.files[0];
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let entry = manifest.find_by_dest(&dest).unwrap();
        assert_eq!(
            app_entry_status(&config, cat, file, entry, &config.env).await,
            FileStatus::UpToDate
        );
        let mut separator = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();
        assert_eq!(report.updated, 0);
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "x"
        );
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v1\n");

        crate::trust::grant_current_for_test(&config, "app/sample").await;
        handle_refresh(&config, "sample", Some("first.txt"), false)
            .await
            .unwrap();
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v2\n");
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "xx"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn automatic_generator_status_reports_refresh_without_execution() {
        let root = crate::test_support::make_temp_dir("shine-refresh-status").await;
        let config = write_fixture(&root, false).await;
        let metadata_path = config.presets_dir().join("app/sample/shine.toml");
        let metadata = fs::read_to_string(&metadata_path)
            .await
            .unwrap()
            .replace("auto = false", "auto = true");
        fs::write(&metadata_path, metadata).await.unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let rows = crate::status::build_app_rows(&config, &categories)
            .await
            .unwrap();
        assert_eq!(rows[0].file_status, FileStatus::GeneratorNotEvaluated);
        assert_eq!(
            fs::read_to_string(config.presets_dir().join("app/sample/first.runs"))
                .await
                .unwrap(),
            "x",
            "read-only status must not execute an automatic generator"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn explicit_generator_evaluation_materializes_desired_content_before_install() {
        let root = crate::test_support::make_temp_dir("shine-generator-preview").await;
        let config = write_fixture(&root, false).await;
        let mut runtime = crate::core_runtime::from_config(&config).await.unwrap();
        runtime.context_mut_for_cli().env = config.env.clone();
        let inspections = runtime
            .inspect_apps_with_options(
                shine_core::runtime::AppInspectionOptions {
                    run_generators: true,
                    categories: vec!["sample".to_string()],
                },
                &mut shine_core::runtime::NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(
            inspections[0].desired_content.as_deref(),
            Some(b"first-v1\n".as_slice())
        );
        assert!(!root.join("dest/first.txt").exists());
        assert_eq!(
            fs::read_to_string(config.presets_dir().join("app/sample/first.runs"))
                .await
                .unwrap(),
            "x"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn explicit_generator_evaluation_updates_status_without_writing_destination() {
        let root = crate::test_support::make_temp_dir("shine-generator-evaluation").await;
        let config = write_fixture(&root, false).await;
        crate::trust::grant_current_for_test(&config, "app/sample").await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let app_dir = config.presets_dir().join("app/sample");
        let destination = root.join("dest/first.txt");
        fs::write(app_dir.join("first.payload"), b"first-v2\n")
            .await
            .unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let (rows, lifecycle, _) =
            crate::status::build_app_rows_with_lifecycle_options(&config, &categories, true)
                .await
                .unwrap();
        assert_eq!(rows[0].file_status, FileStatus::UpdateAvail);
        assert!(!rows[0].upgrade_available);
        assert_eq!(rows[0].refresh_sources, ["first.txt"]);
        assert_eq!(rows[0].status_text, "refresh available");
        assert_eq!(
            lifecycle.outcomes[0].diagnostic_codes,
            ["app_manual_refresh_required"]
        );
        assert_eq!(fs::read(&destination).await.unwrap(), b"first-v1\n");
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "xx",
            "explicit evaluation must execute the selected generator exactly once"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_selector_and_force_preserve_other_generated_files() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, true).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let app_dir = config.presets_dir().join("app/sample");
        let first_dest = root.join("dest/first.txt");
        let second_dest = root.join("dest/second.txt");
        fs::write(app_dir.join("first.payload"), b"first-v2\n")
            .await
            .unwrap();
        fs::write(app_dir.join("second.payload"), b"second-v2\n")
            .await
            .unwrap();
        fs::write(&first_dest, b"user edit\n").await.unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        assert!(
            handle_refresh(&config, "sample", Some("first.txt"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&first_dest).await.unwrap(), b"user edit\n");
        handle_refresh(&config, "sample", Some("first.txt"), true)
            .await
            .unwrap();
        assert_eq!(fs::read(&first_dest).await.unwrap(), b"first-v2\n");
        assert_eq!(fs::read(&second_dest).await.unwrap(), b"second-v1\n");
        assert_eq!(
            fs::read_to_string(app_dir.join("second.runs"))
                .await
                .unwrap(),
            "x",
            "single-file refresh must not run other generators"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_keeps_last_good_file_and_continues_after_generator_failure() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, true).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let app_dir = config.presets_dir().join("app/sample");
        let first_dest = root.join("dest/first.txt");
        let second_dest = root.join("dest/second.txt");
        fs::write(app_dir.join("first.sh"), b"#!/bin/sh\nexit 1\n")
            .await
            .unwrap();
        fs::write(app_dir.join("second.payload"), b"second-v2\n")
            .await
            .unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        assert!(
            handle_refresh(&config, "sample", None, false)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(&first_dest).await.unwrap(),
            b"first-v1\n",
            "failed generator must retain the last-known-good file"
        );
        assert_eq!(
            fs::read(&second_dest).await.unwrap(),
            b"second-v2\n",
            "a failed generator must not prevent later selected files refreshing"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_requires_the_generator_condition_env() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let mut config = write_fixture(&root, false).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        config.env.remove("SOURCE_URL");
        let dest = root.join("dest/first.txt");

        assert!(
            handle_refresh(&config, "sample", Some("first.txt"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v1\n");
        fs::remove_dir_all(root).await.unwrap();
    }
}
