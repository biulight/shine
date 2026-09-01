#[cfg(test)]
use super::profile::remove_path_from_shell_config;
use super::report::{
    shell_cache_remove_summary_parts, style_dim, style_symbol, unlink_report_summary_parts,
};
use crate::config::Config;
use crate::output;
use crate::presentation::{LifecycleReporter, PresentationEvent, TerminalRenderer};
use anyhow::Result;
use shine_core::lifecycle::LifecycleOperation;
use shine_core::lifecycle::LifecycleResultV1;
#[cfg(test)]
use shine_core::lifecycle::{LifecycleEffect, LifecycleStatus};
use shine_core::runtime::{PlanningInputVersions, ShellPlanRequest};

pub async fn handle_uninstall(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    handle_uninstall_approved(config, target, purge, dry_run, true).await
}

pub async fn handle_uninstall_approved(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, target, purge, dry_run, yes, &mut renderer)
        .await
        .map(|_| ())
}

#[cfg(test)]
pub(crate) async fn handle_uninstall_with_result(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<LifecycleResultV1> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, target, purge, dry_run, true, &mut renderer).await
}

async fn handle_uninstall_with_reporter(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
    yes: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    for line in crate::config::presets_note_lines(config) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    if dry_run {
        reporter.emit(PresentationEvent::stdout(style_dim(
            "[dry-run] No files will be modified.",
        )));
    }
    let reviewed = if dry_run {
        None
    } else {
        crate::lifecycle_plan::review_plans(
            config,
            [crate::lifecycle_plan::LifecyclePlanRequest::shell(
                ShellPlanRequest {
                    operation: LifecycleOperation::Uninstall,
                    target: target.map(str::to_string),
                    force: false,
                    purge,
                    input_versions: PlanningInputVersions::default(),
                },
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
    let core_report = if let Some(reviewed) = &reviewed {
        runtime
            .uninstall_shells_approved(
                match &reviewed.request {
                    crate::lifecycle_plan::LifecyclePlanRequest::Shell(request) => request.clone(),
                    _ => unreachable!("reviewed Shell Plan"),
                },
                &reviewed.approval,
            )
            .await?
    } else {
        runtime
            .preview_uninstall_shells(shine_core::runtime::ShellUninstallRequest {
                target: target.map(str::to_string),
                dry_run,
                purge,
            })
            .await?
    };
    reporter.emit(PresentationEvent::stdout(output::summary_line_text(
        "Bin Links",
        &unlink_report_summary_parts(&core_report.links),
    )));
    if !config.is_external_presets {
        reporter.emit(PresentationEvent::stdout(output::summary_line_text(
            "Shell Presets",
            &shell_cache_remove_summary_parts(&core_report.cache),
        )));
    }
    if purge && !dry_run && !config.is_external_presets {
        reporter.emit(PresentationEvent::stdout(format!(
            "  {}  {}",
            style_symbol("✓"),
            style_dim("managed directories purged (if empty)"),
        )));
    }
    if let Some(profile) = &core_report.profile {
        for path in &profile.config_paths {
            reporter.emit(PresentationEvent::stdout(format!(
                "Shell config ({}): shine entry removed",
                crate::path_display::format_home(path, &config.home_dir)
            )));
        }
        if let Some(path) = &profile.managed_profile {
            reporter.emit(PresentationEvent::stdout(format!(
                "Shell profile ({}): removed",
                crate::path_display::format_home(path, &config.home_dir)
            )));
        }
    }
    Ok(core_report.lifecycle)
}

#[cfg(test)]
mod tests {
    use super::super::ShellType;
    use super::super::install::handle_install;
    use super::super::profile::{append_path_to_shell_config, managed_shell_profile_path};
    use super::*;
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell").await
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_installed_sibling() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        handle_install(&config, Some("utils/shine-theme-sync"), false)
            .await
            .unwrap();
        handle_uninstall(&config, Some("utils/shine-env-export"), false, false)
            .await
            .unwrap();

        let removed = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        assert!(!removed.exists());
        assert!(sibling.exists());

        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_none());
        assert!(manifest.find("shell/utils/shine-theme-sync").is_some());

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(!profile.contains(&wrapper_marker("shine-env-export", &config.shell_type)));
        assert!(profile.contains(&wrapper_marker("shine-theme-sync", &config.shell_type)));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_shared_rendered_file() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"shared.sh\"\ntarget = \"one\"\ntransforms = [\"template\"]\n[files.permissions]\nschema_version = 1\n\n[[files]]\nsource = \"shared.sh\"\ntarget = \"two\"\ntransforms = [\"template\"]\n[files.permissions]\nschema_version = 1\n",
        )
        .await
        .unwrap();
        fs::write(category.join("shared.sh"), b"#!/bin/sh\necho shared\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom/one"), false)
            .await
            .unwrap();
        handle_install(&config, Some("custom/two"), false)
            .await
            .unwrap();
        let rendered = config.rendered_dir().join("shell/custom/shared.sh");
        assert!(rendered.exists());

        handle_uninstall(&config, Some("custom/one"), false, false)
            .await
            .unwrap();

        assert!(
            rendered.exists(),
            "installed sibling still uses rendered file"
        );
        assert!(
            crate::bin_links::command_path_for_name(config.bin_dir(), std::ffi::OsStr::new("two"))
                .exists()
        );
        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/custom/one").is_none());
        assert_eq!(
            manifest.find("shell/custom/two").unwrap().rendered_path,
            rendered
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_dry_run_preserves_launcher_and_manifest() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();

        let result =
            handle_uninstall_with_result(&config, Some("utils/shine-env-export"), false, true)
                .await
                .unwrap();

        assert_eq!(result.outcomes[0].status, LifecycleStatus::Previewed);
        assert!(
            result.outcomes[0]
                .effects
                .contains(&LifecycleEffect::CacheRemovePreviewed)
        );

        let command = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        assert!(command.exists());
        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_some());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_foreign_command_entry() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        let command = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        shine_core::runtime::unlink_managed_command_with_host(
            &shine_core::runtime::RealHost,
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
            &[config.presets_dir().join("shell/utils")],
            false,
        )
        .await
        .unwrap();
        fs::write(&command, b"user-owned command\n").await.unwrap();

        let update = super::super::install::collect_update_lifecycle_result(&config)
            .await
            .unwrap();
        let pending = update
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "shell/utils/shine-env-export")
            .unwrap();
        assert_eq!(pending.status, LifecycleStatus::Conflict);

        let mut separator = crate::output::SectionSeparator::new();
        let error = super::super::install::handle_upgrade_installed_target_with_result(
            &config,
            Some("utils"),
            false,
            &mut separator,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Plan is blocked"));
        assert_eq!(
            fs::read_to_string(&command).await.unwrap(),
            "user-owned command\n"
        );

        let result =
            handle_uninstall_with_result(&config, Some("utils/shine-env-export"), false, false)
                .await
                .unwrap();

        assert_eq!(result.outcomes[0].status, LifecycleStatus::Conflict);
        assert!(
            result.outcomes[0]
                .effects
                .contains(&LifecycleEffect::UserResourcePreserved)
        );

        assert_eq!(
            fs::read_to_string(&command).await.unwrap(),
            "user-owned command\n"
        );
        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_none());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    fn wrapper_marker(command: &str, shell: &ShellType) -> String {
        match shell {
            ShellType::PowerShell => format!("\nfunction {command} {{ . (Join-Path $shineBin"),
            ShellType::Fish => format!("\nfunction {command}"),
            _ => format!("\n{command}() {{ source"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_purge_removes_managed_dirs_but_not_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        handle_uninstall(&config, None, true, false).await.unwrap();

        assert!(!config.bin_dir().exists(), "bin_dir should be purged");
        assert!(
            !config.presets_dir().join("shell").exists(),
            "shell presets dir should be purged"
        );
        // config.toml must never be removed by uninstall
        assert!(
            config.presets_dir().parent().is_some(),
            "shine root still accessible"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        let preset_path = config.presets_dir().join("shell/proxy/set_proxy.sh");
        assert!(preset_path.exists());

        handle_uninstall(&config, None, false, true).await.unwrap();

        assert!(preset_path.exists(), "dry-run must not remove preset files");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_clears_sentinel_from_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
        remove_path_from_shell_config(&config).await.unwrap();

        let config_path =
            super::super::get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            !content.contains(super::super::SENTINEL_START),
            "sentinel should be gone after remove"
        );
    }

    #[tokio::test]
    async fn remove_is_no_op_when_config_missing() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        // No install — config file doesn't exist
        remove_path_from_shell_config(&config).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_does_not_modify_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        let config_path =
            super::super::get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let before = fs::read_to_string(&config_path).await.unwrap();
        let profile_path = managed_shell_profile_path(&config);
        let profile_before = fs::read_to_string(&profile_path).await.unwrap();

        handle_uninstall(&config, None, false, true).await.unwrap();

        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(before, after, "dry-run must not touch shell config");
        let profile_after = fs::read_to_string(&profile_path).await.unwrap();
        assert_eq!(
            profile_before, profile_after,
            "dry-run must not touch managed shell profile"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_category_keeps_agent_launcher_and_prunes_source_wrappers() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();
        handle_install(&config, Some("proxy"), false).await.unwrap();

        handle_uninstall(&config, Some("proxy"), false, false)
            .await
            .unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(!profile.contains(&wrapper_marker("ccenv", &config.shell_type)));
        assert!(
            !profile.contains(&wrapper_marker("setproxy", &config.shell_type)),
            "removed category wrapper should be pruned: {profile}"
        );
        assert!(
            !profile.contains(&wrapper_marker("usetproxy", &config.shell_type)),
            "removed category wrapper should be pruned: {profile}"
        );
        let ccenv = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("ccenv"),
        );
        assert!(ccenv.exists(), "remaining Bun launcher should be kept");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_uninstall_preserves_disk_scripts() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"my_tool.sh\"\ntarget = \"my_tool\"\n[files.permissions]\nschema_version = 1\n",
        )
        .await
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        assert!(config.bin_dir().join("my_tool").exists());

        handle_uninstall(&config, Some("custom"), false, false)
            .await
            .unwrap();

        // User-owned script must survive uninstall.
        assert!(script.exists(), "user script must not be deleted");
        // Bin symlink should be gone.
        assert!(
            !config.bin_dir().join("my_tool").exists(),
            "bin link should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
