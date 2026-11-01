mod build;
mod info;
mod install;
mod metadata;
mod recovery;
mod refresh;
mod report;
mod uninstall;
mod upgrade;

pub use build::{handle_build_approved, handle_unbuild_approved};
#[doc(hidden)]
pub use info::handle_list_with_presets_note;
pub use info::{handle_info, handle_list};
pub use install::handle_install_approved;
pub use metadata::{
    AppCategory, AppDestinationRoot, AppFile, AppGenerator, AppHook, AppListMode,
    load_active_categories, load_embedded_categories, load_installed_categories,
};
pub use recovery::handle_recover_approved;
pub use refresh::handle_refresh_approved;
pub use uninstall::handle_uninstall_approved;
pub use upgrade::AppUpgradeReport;
#[cfg(all(test, unix))]
pub(crate) use upgrade::handle_upgrade_installed_target_with_result;
pub(crate) use upgrade::{
    handle_upgrade_installed_target_with_result_approved,
    handle_upgrade_installed_with_output_with_result_prepared,
};

use anyhow::{Context, Result};
#[cfg(test)]
use std::path::PathBuf;
const APP_TEMPLATE: &str = r#"# App preset metadata for shine.
metadata_schema_version = 2
description = "My app configuration."
dest = "~/.config/my-app"
# Optional category platform destination. Exact OS keys override the Unix fallback:
# dest = { macos = "~/Library/Application Support/My App", linux = "~/.config/my-app", windows = "~/AppData/Roaming/My App", unix = "~/.config/my-app" }
# Optional author capability notes. Shine derives managed file operations from typed metadata;
# these notes do not restrict arbitrary code:
# [permissions]
# schema_version = 1
# commands = ["bun"]
# network = [{ scope = "host", host = "api.example.com" }]
# Declare script inputs at their entry: env = [{ source = "API_TOKEN", sensitivity = "secret" }]

[[files]]
source = "config.toml"
target = "config.toml"
# Optional: platforms = ["macos"] # exact: macos/linux/windows; unix groups macOS + Linux
# Optional per-file override:
# dest = { base = "data-dir", path = "com.example.my-app" }
description = "Main application config"
display_name = "config.toml"
# Known transforms: "template", "jsonc-to-json".
transforms = []
# Optional generated source. The static `source` above is the fallback.
# `auto = false` disables implicit status/upgrade runs; use `app refresh`.
# generator = { script = "generate.ts", runtime = "bun", env = [{ source = 'SOURCE_URL', sensitivity = 'plain' }], when_env = "SOURCE_URL", auto = false }
"#;

pub async fn handle_init_template(force: bool, unrestricted: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let template = init_template(unrestricted);
    let (path, overwritten) =
        shine_core::init_template::write_shine_toml_template(&dir, force, &template)?;
    if overwritten {
        println!(
            "Updated app preset template: {}",
            crate::output::escape_controls(&path.display().to_string())
        );
    } else {
        println!(
            "Created app preset template: {}",
            crate::output::escape_controls(&path.display().to_string())
        );
    }
    Ok(())
}

fn init_template(unrestricted: bool) -> String {
    if unrestricted {
        format!(
            "{APP_TEMPLATE}\n[permissions]\nschema_version = 2\nopaque_code = \"unrestricted\"\n"
        )
    } else {
        APP_TEMPLATE.to_string()
    }
}

#[cfg(test)]
#[test]
fn unrestricted_init_template_is_valid_schema_v2() {
    let manifest: toml::Value = toml::from_str(&init_template(true)).unwrap();
    let permissions = manifest.get("permissions").unwrap();
    assert_eq!(
        permissions
            .get("schema_version")
            .and_then(toml::Value::as_integer),
        Some(2)
    );
    assert_eq!(
        permissions.get("opaque_code").and_then(toml::Value::as_str),
        Some("unrestricted")
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::config::Config;
    #[cfg(unix)]
    use crate::install_core::manifest::AppManifest;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-apps").await
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn surge_runtime_actions_are_unavailable_outside_macos() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        for error in [
            info::handle_info(&config, "surge", false, false)
                .await
                .unwrap_err(),
            build::handle_build_approved(&config, "surge", true)
                .await
                .unwrap_err(),
            refresh::handle_refresh_approved(&config, "surge", None, false, true)
                .await
                .unwrap_err(),
            install::handle_install_approved(&config, Some("surge"), false, false, true)
                .await
                .unwrap_err(),
        ] {
            assert!(
                error
                    .to_string()
                    .contains("app preset category not found: surge"),
                "unexpected error: {error:#}"
            );
        }

        assert!(
            !dir.join("Library/Application Support/Surge/Profiles")
                .exists()
        );
        assert!(!dir.join("presets/app/surge").exists());
        assert!(!dir.join("env.toml").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    async fn write_external_sample_app(dir: &std::path::Path, body: &[u8]) {
        write_external_sample_app_with_extra(dir, body, None).await;
    }

    #[cfg(unix)]
    async fn write_external_sample_app_with_extra(
        dir: &std::path::Path,
        body: &[u8],
        extra_body: Option<&[u8]>,
    ) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let mut manifest = "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n".to_string();
        if extra_body.is_some() {
            manifest.push_str(
                "\n[[files]]\nsource = \"theme.conf\"\ntarget = \"themes/theme.conf\"\ntransforms = [\"template\"]\n",
            );
        }
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
        if let Some(extra_body) = extra_body {
            fs::write(cat_dir.join("theme.conf"), extra_body)
                .await
                .unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn uninstall_remains_manifest_driven_after_category_becomes_macos_only() {
        let dir = make_temp_dir().await;
        write_external_sample_app(&dir, b"{\"enabled\":true}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        install::handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let destination = dir.join(".config/sample/daemon.json");
        assert!(destination.exists());

        fs::write(
            dir.join("presets/app/sample/shine.toml"),
            b"dest = { macos = \"~/Library/Sample\" }\n[[files]]\nsource = \"daemon.jsonc\"\n",
        )
        .await
        .unwrap();
        uninstall::handle_uninstall_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();

        assert!(!destination.exists());
        assert!(
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    async fn write_external_sample_app_with_post_upgrade(
        dir: &std::path::Path,
        body: &[u8],
        script_path: &std::path::Path,
        marker_path: &std::path::Path,
    ) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let manifest = format!(
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\npost_upgrade = {{ command = \"/bin/sh\", args = [\"{}\", \"{}\"] }}\n\n[permissions]\nschema_version = 1\ncommands = [\"/bin/sh\"]\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n",
            script_path.display(),
            marker_path.display()
        );
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
    }

    #[cfg(unix)]
    async fn write_hook_script(path: &std::path::Path) {
        fs::write(path, "#!/bin/sh\nprintf x >> \"$1\"\n")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn init_template_creates_parseable_app_metadata() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();

        let (path, overwritten) =
            shine_core::init_template::write_shine_toml_template(&cat_dir, false, APP_TEMPLATE)
                .unwrap();
        fs::write(cat_dir.join("config.toml"), b"name = \"sample\"\n")
            .await
            .unwrap();

        let config = Config::new_for_test(&dir);
        let categories = metadata::load_installed_categories(&config, Some("sample"))
            .await
            .unwrap();

        assert_eq!(path, cat_dir.join("shine.toml"));
        assert!(!overwritten);
        assert_eq!(categories.len(), 1);
        assert_eq!(
            categories[0].description.as_deref(),
            Some("My app configuration.")
        );
        assert_eq!(
            categories[0].destination_root.as_deref(),
            Some("~/.config/my-app")
        );
        assert!(categories[0].permissions.is_none());
        assert_eq!(
            categories[0].files[0].source_rel,
            PathBuf::from("config.toml")
        );
        assert_eq!(
            categories[0].files[0].target_rel,
            PathBuf::from("config.toml")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_refuses_existing_file_unless_forced() {
        let dir = make_temp_dir().await;
        fs::write(dir.join("shine.toml"), b"old").await.unwrap();

        let err = shine_core::init_template::write_shine_toml_template(&dir, false, APP_TEMPLATE)
            .unwrap_err();
        assert!(
            err.to_string().contains("use --force to overwrite"),
            "unexpected error: {err:#}"
        );
        assert_eq!(fs::read(dir.join("shine.toml")).await.unwrap(), b"old");

        let (_path, overwritten) =
            shine_core::init_template::write_shine_toml_template(&dir, true, APP_TEMPLATE).unwrap();
        assert!(overwritten);
        let content = fs::read_to_string(dir.join("shine.toml")).await.unwrap();
        assert!(content.contains("dest = \"~/.config/my-app\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_up_to_date_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(
            &dir,
            b"{\n  // proxy\n  \"proxy\": \"@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@\"\n}\n",
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 0, "up-to-date app config must not update");
        assert_eq!(report.skipped, 1, "up-to-date app config should be skipped");
        assert_eq!(fs::read(&dest).await.unwrap(), before);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_updates_app_config_when_source_changes() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();
        let manifest_before = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let hash_before = manifest_before.entries[0].content_hash;

        write_external_sample_app(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
        )
        .await;
        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 1, "changed source should update");
        assert_eq!(report.skipped, 0);
        assert_ne!(fs::read(&dest).await.unwrap(), before);
        let manifest_after = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert_ne!(manifest_after.entries[0].content_hash, hash_before);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn targeted_upgrade_does_not_mutate_other_app_categories() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"one\"\n}\n").await;
        let other_dir = dir.join("presets/app/other");
        fs::create_dir_all(&other_dir).await.unwrap();
        fs::write(
            other_dir.join("shine.toml"),
            "description = \"Other app\"\ndest = \"~/.config/other\"\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"config.json\"\ntarget = \"config.json\"\n",
        )
        .await
        .unwrap();
        fs::write(other_dir.join("config.json"), b"{\"value\":1}\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        handle_install_approved(&config, Some("other"), false, false, true)
            .await
            .unwrap();

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"two\"\n}\n").await;
        fs::write(other_dir.join("config.json"), b"{\"value\":2}\n")
            .await
            .unwrap();
        let other_dest = dir.join(".config/other/config.json");
        let other_before = fs::read(&other_dest).await.unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, lifecycle) = handle_upgrade_installed_target_with_result(
            &config,
            Some("sample"),
            false,
            false,
            &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 1);
        assert!(lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "app/sample"
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome.resource.as_deref() == Some("daemon.jsonc")
        }));
        assert_eq!(fs::read(&other_dest).await.unwrap(), other_before);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn structured_app_lifecycle_covers_update_upgrade_and_target_isolation() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\"value\":\"initial\"}\n").await;
        let other_dir = dir.join("presets/app/other");
        fs::create_dir_all(&other_dir).await.unwrap();
        fs::write(
            other_dir.join("shine.toml"),
            "description = \"Other app\"\ndest = \"~/.config/other\"\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"config.json\"\ntarget = \"config.json\"\n",
        )
        .await
        .unwrap();
        fs::write(other_dir.join("config.json"), b"{\"value\":1}\n")
            .await
            .unwrap();

        let sample_destination = dir.join(".config/sample/daemon.json");
        fs::create_dir_all(sample_destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(&sample_destination, b"user original\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        let install = install::handle_install_with_result(&config, Some("sample"), false, false)
            .await
            .unwrap();
        assert!(install.outcomes.iter().any(|outcome| {
            outcome.target == "app/sample"
                && outcome.resource.as_deref() == Some("daemon.jsonc")
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome
                    .effects
                    .contains(&shine_core::lifecycle::LifecycleEffect::BackupCreated)
        }));
        install::handle_install_with_result(&config, Some("other"), false, false)
            .await
            .unwrap();

        write_external_sample_app(&dir, b"{\"value\":\"private-source-token\"}\n").await;
        fs::write(other_dir.join("config.json"), b"{\"value\":2}\n")
            .await
            .unwrap();
        let other_destination = dir.join(".config/other/config.json");
        let other_before = fs::read(&other_destination).await.unwrap();
        let other_manifest_before =
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .into_iter()
                .find(|entry| entry.source == "app/other/config.json")
                .unwrap();

        let categories = metadata::load_active_categories(&config, None)
            .await
            .unwrap();
        let (_rows, update, _) =
            crate::status::build_app_rows_with_lifecycle_options(&config, &categories, false)
                .await
                .unwrap();
        let pending = update
            .outcomes
            .iter()
            .find(|outcome| {
                outcome.target == "app/sample"
                    && outcome.resource.as_deref() == Some("daemon.jsonc")
            })
            .unwrap();
        assert_eq!(
            pending.status,
            shine_core::lifecycle::LifecycleStatus::Pending
        );
        assert_eq!(
            pending.effects,
            [
                shine_core::lifecycle::LifecycleEffect::ResourceWritePreviewed,
                shine_core::lifecycle::LifecycleEffect::ReceiptWritePreviewed,
            ]
        );
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(!serialized.contains(&dir.display().to_string()));
        assert!(!serialized.contains("private-source-token"));

        let mut separator = crate::output::SectionSeparator::new();
        let (report, upgrade) = upgrade::handle_upgrade_installed_target_with_result(
            &config,
            Some("sample"),
            false,
            false,
            &mut separator,
        )
        .await
        .unwrap();
        assert_eq!(report.updated, 1);
        assert!(upgrade.outcomes.iter().any(|outcome| {
            outcome.target == "app/sample"
                && outcome.resource.as_deref() == Some("daemon.jsonc")
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
        }));
        assert_eq!(fs::read(&other_destination).await.unwrap(), other_before);
        let other_manifest_after =
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .into_iter()
                .find(|entry| entry.source == "app/other/config.json")
                .unwrap();
        assert_eq!(
            toml::to_string(&other_manifest_after).unwrap(),
            toml::to_string(&other_manifest_before).unwrap()
        );

        let categories = metadata::load_active_categories(&config, None)
            .await
            .unwrap();
        let (_rows, after_upgrade, _) =
            crate::status::build_app_rows_with_lifecycle_options(&config, &categories, false)
                .await
                .unwrap();
        assert!(after_upgrade.outcomes.iter().any(|outcome| {
            outcome.target == "app/sample"
                && outcome.resource.as_deref() == Some("daemon.jsonc")
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Unchanged
        }));
        assert!(after_upgrade.outcomes.iter().any(|outcome| {
            outcome.target == "app/other"
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Pending
        }));

        let uninstall =
            uninstall::handle_uninstall_with_result(&config, Some("sample"), false, false)
                .await
                .unwrap();
        assert!(uninstall.outcomes.iter().any(|outcome| {
            outcome.target == "app/sample"
                && outcome.resource.as_deref() == Some("daemon.jsonc")
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome
                    .effects
                    .contains(&shine_core::lifecycle::LifecycleEffect::BackupRestored)
        }));
        assert_eq!(
            fs::read(&sample_destination).await.unwrap(),
            b"user original\n"
        );
        assert!(other_destination.exists());
        assert!(
            AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.source == "app/other/config.json")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_runs_post_upgrade_hook_after_file_update_when_allowed() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        assert!(
            !marker.exists(),
            "post-upgrade hook must not run during install"
        );
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
            &script,
            &marker,
        )
        .await;
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        let mut sep = crate::output::SectionSeparator::new();
        let (report, lifecycle) =
            handle_upgrade_installed_target_with_result(&config, None, false, false, &mut sep)
                .await
                .unwrap();

        assert_eq!(report.updated, 1);
        assert!(lifecycle.outcomes.iter().any(|outcome| {
            outcome.resource.as_deref() == Some("hook:post-upgrade")
                && outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome
                    .effects
                    .contains(&shine_core::lifecycle::LifecycleEffect::CodeExecuted)
        }));
        assert_eq!(fs::read_to_string(&marker).await.unwrap(), "x");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_does_not_run_post_upgrade_hook_when_unchanged() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        crate::trust::grant_current_for_test(&config, "app/sample").await;

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 0);
        assert!(!marker.exists(), "unchanged config must not run hook");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn external_post_upgrade_hook_is_skipped_without_opt_in() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
            &script,
            &marker,
        )
        .await;

        let mut sep = crate::output::SectionSeparator::new();
        let error =
            handle_upgrade_installed_target_with_result(&config, None, false, false, &mut sep)
                .await
                .unwrap_err();

        assert!(error.to_string().contains("Plan is blocked"));
        assert!(
            !marker.exists(),
            "external hook must be skipped without a matching scoped trust grant"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_installs_new_app_file_from_installed_category() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        write_external_sample_app_with_extra(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            Some(b"background = @@GHOSTTY_BG_LIGHT@@\n"),
        )
        .await;

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        let new_dest = dir.join(".config/sample/themes/theme.conf");
        assert_eq!(report.updated, 1, "new app file should be installed");
        assert_eq!(report.updated_categories, 1);
        assert_eq!(
            report.skipped, 1,
            "existing up-to-date file should be skipped"
        );
        assert_eq!(
            fs::read(&new_dest).await.unwrap(),
            b"background = \n",
            "new file should be transformed before install"
        );
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_some(),
            "new app file should be tracked in manifest"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_new_app_file_when_destination_is_unmanaged() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        write_external_sample_app_with_extra(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            Some(b"background = @@GHOSTTY_BG_LIGHT@@\n"),
        )
        .await;
        let new_dest = dir.join(".config/sample/themes/theme.conf");
        fs::create_dir_all(new_dest.parent().unwrap())
            .await
            .unwrap();
        fs::write(&new_dest, b"user-owned\n").await.unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let error = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("Plan is blocked"));
        assert_eq!(fs::read(&new_dest).await.unwrap(), b"user-owned\n");
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_none(),
            "unmanaged destination should not be added to manifest"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_removes_unmodified_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, true, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 1, "stale cleanup should count as a change");
        assert_eq!(report.skipped, 0);
        assert!(!dest.exists(), "unmodified stale file should be removed");
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "stale manifest entry should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_removes_manifest_entry_when_destination_is_missing() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_file(&dest).await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, true, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 1, "manifest cleanup should count as change");
        assert_eq!(report.skipped, 0);
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "missing stale destination should be removed from manifest"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_without_prune_keeps_stale_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert!(dest.exists(), "stale file should be left in place");
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "stale manifest entry should remain without prune"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_keeps_user_modified_file() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, true, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.user_modified, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "user-modified stale entry should remain tracked"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_allows_renamed_source_to_reinstall_same_destination() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"old\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let cat_dir = dir.join("presets/app/sample");
        fs::write(
            cat_dir.join("shine.toml"),
            b"description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = \"daemon-renamed.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"jsonc-to-json\"]\n",
        )
        .await
        .unwrap();
        fs::write(
            cat_dir.join("daemon-renamed.jsonc"),
            b"{\n  \"proxy\": \"new\"\n}\n",
        )
        .await
        .unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, true, false, true, &mut sep,
        )
        .await
        .unwrap();

        let dest = dir.join(".config/sample/daemon.json");
        assert_eq!(
            report.updated, 2,
            "cleanup plus reinstall should change state"
        );
        assert_eq!(report.updated_categories, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(
            fs::read(&dest).await.unwrap(),
            b"{\n  \"proxy\": \"new\"\n}\n"
        );
        let manifest = AppManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
            .await
            .unwrap();
        let entry = manifest.find_by_dest(&dest).unwrap();
        assert_eq!(entry.source, "app/sample/daemon-renamed.jsonc");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_user_modified_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install_approved(&config, Some("sample"), false, false, true)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();

        let mut sep = crate::output::SectionSeparator::new();
        let (report, _) = handle_upgrade_installed_target_with_result_approved(
            &config, None, false, false, true, &mut sep,
        )
        .await
        .unwrap();

        assert_eq!(
            report.updated, 0,
            "user-modified app config must not update"
        );
        assert_eq!(report.skipped, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}

pub(crate) mod secret_files;
