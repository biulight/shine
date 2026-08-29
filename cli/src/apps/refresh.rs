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
        colors::dim(&format!(
            "Refresh complete: {updated} updated, {unchanged} unchanged, {failed} failed"
        ))
    );
    if failed > 0 {
        bail!("{failed} generated app file(s) failed to refresh");
    }
    Ok(())
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
    use std::os::unix::fs::PermissionsExt;
    use tokio::fs;

    async fn write_fixture(root: &Path, two_files: bool) -> Config {
        let mut config = Config::new_for_test(root);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
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
environment = [{{ name = "SOURCE_URL", sensitivity = "plain" }}]

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
