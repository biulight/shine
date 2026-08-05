use super::install::installed_source_commands;
use super::profile::{
    remove_managed_shell_profile, remove_path_from_shell_config, write_managed_shell_profile,
};
use super::report::{remove_report_summary_parts, unlink_report_summary_parts};
use crate::config::Config;
use crate::output;
use anyhow::{Context, Result};

pub async fn handle_uninstall(
    config: &Config,
    category: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    if dry_run {
        println!(
            "{}",
            crate::colors::dim("[dry-run] No files will be modified.")
        );
    }

    // When a category is given, scope removal to that category's subdirectory.
    let managed_presets_root = match category {
        Some(cat) => config.presets_dir().join("shell").join(cat),
        None => config.presets_dir().to_path_buf(),
    };
    let managed_rendered_root = match category {
        Some(cat) => config.rendered_dir().join("shell").join(cat),
        None => config.rendered_dir().join("shell"),
    };
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_owned(),
    };

    // Remove symlinks pointing to presets_dir (old-style) or rendered_dir (new-style).
    let unlink_presets =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_presets_root, dry_run).await?;
    let unlink_rendered =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_rendered_root, dry_run).await?;
    let managed_installed_root = match category {
        Some(cat) => config.installed_shell_dir().join(cat),
        None => config.installed_shell_dir(),
    };
    let unlink_installed =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_installed_root, dry_run)
            .await?;
    let unlink_report = crate::bin_links::UnlinkReport {
        removed: [
            unlink_presets.removed,
            unlink_rendered.removed,
            unlink_installed.removed,
        ]
        .concat(),
        skipped: [
            unlink_presets.skipped,
            unlink_rendered.skipped,
            unlink_installed.skipped,
        ]
        .concat(),
    };
    output::summary_line("Bin Links", &unlink_report_summary_parts(&unlink_report));

    // When the user has a custom presets directory, the source files are theirs —
    // only remove the embedded-managed files when using the default directory.
    if !config.is_external_presets {
        let remove_report =
            crate::presets::remove_prefix(&prefix, config.presets_dir(), dry_run).await?;
        output::summary_line(
            "Shell Presets",
            &remove_report_summary_parts(&remove_report),
        );
    }

    // Only purge managed directories when using the default presets directory.
    // Never delete a user-configured external folder.
    if purge && !dry_run && !config.is_external_presets {
        let purge_dir = match category {
            Some(cat) => config.presets_dir().join("shell").join(cat),
            None => config.presets_dir().join("shell"),
        };
        if purge_dir.exists() {
            tokio::fs::remove_dir_all(&purge_dir)
                .await
                .with_context(|| format!("removing presets directory: {purge_dir:?}"))?;
        }
        if category.is_none() {
            // remove_dir only succeeds if empty — treat non-empty as benign
            let _ = tokio::fs::remove_dir(config.presets_dir()).await;
            let _ = tokio::fs::remove_dir(config.bin_dir()).await;
        }
        println!(
            "  {}  {}",
            crate::colors::symbol("✓"),
            crate::colors::dim("managed directories purged (if empty)"),
        );
    }

    // Remove rendered_dir files — always shine-managed regardless of external-presets mode.
    if !dry_run && managed_rendered_root.exists() {
        tokio::fs::remove_dir_all(&managed_rendered_root)
            .await
            .with_context(|| {
                format!("removing rendered dir: {}", managed_rendered_root.display())
            })?;
    }

    if !dry_run && managed_installed_root.exists() {
        tokio::fs::remove_dir_all(&managed_installed_root)
            .await
            .with_context(|| {
                format!(
                    "removing installed shell snapshot: {}",
                    managed_installed_root.display()
                )
            })?;
    }

    if !dry_run {
        let mut manifest = super::deployment::ShellManifest::load(config).await?;
        if let Some(category) = category {
            manifest.remove_category(category);
        } else {
            manifest.entries.clear();
        }
        manifest.save(config).await?;
    }

    if !dry_run {
        if category.is_none() {
            // Only remove the PATH sentinel when uninstalling all shell presets.
            remove_path_from_shell_config(config).await?;
            remove_managed_shell_profile(config).await?;
        } else {
            let remaining_source_commands = installed_source_commands(config).await?;
            write_managed_shell_profile(config, &remaining_source_commands).await?;
        }
    }

    Ok(())
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
