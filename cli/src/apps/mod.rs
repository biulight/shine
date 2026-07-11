mod annotation;
mod info;
mod install;
mod json_merge;
mod metadata;
mod report;
mod uninstall;
mod upgrade;

pub use info::{handle_info, handle_list};
pub use install::handle_install;
pub use metadata::{
    AppCategory, AppFile, AppHook, AppListMode, load_active_categories, load_embedded_categories,
    load_installed_categories,
};
pub use uninstall::handle_uninstall;
pub use upgrade::{AppUpgradeReport, handle_upgrade_installed};

use crate::config::Config;
use crate::install_core::manifest::{self, AppEntry, AppInstallStrategy, hash_content};
use crate::install_core::{file_ops, transforms};
use anyhow::{Context, Result};
use file_ops::{InstallOutcome, UninstallOutcome};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
const APP_TEMPLATE: &str = r#"# App preset metadata for shine.
description = "My app configuration."
dest = "~/.config/my-app"

[[files]]
source = "config.toml"
target = "config.toml"
description = "Main application config"
display_name = "config.toml"
# Known transforms: "template", "jsonc-to-json".
transforms = []
"#;

pub async fn handle_init_template(force: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let (path, overwritten) =
        utils::init_template::write_shine_toml_template(&dir, force, APP_TEMPLATE)?;
    if overwritten {
        println!("Updated app preset template: {}", path.display());
    } else {
        println!("Created app preset template: {}", path.display());
    }
    Ok(())
}

/// Hash the effective install content for `file` — applies transforms if declared.
///
/// Returns `None` when the source cannot be read (e.g. not yet extracted).
pub async fn source_bytes_for_file(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env: &BTreeMap<String, String>,
) -> Option<Vec<u8>> {
    let raw = if config.is_external_presets {
        let path = config.preset_path(Path::new("app").join(&cat.name).join(&file.source_rel));
        tokio::fs::read(&path).await.ok()?
    } else {
        let key = format!("app/{}/{}", cat.name, file.source_rel.display());
        crate::presets::read_asset_bytes(&key)?
    };

    if file.transforms.is_empty() {
        Some(raw)
    } else {
        transforms::apply(&file.transforms, &raw, env).ok()
    }
}

pub async fn source_hash_for_file(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env: &BTreeMap<String, String>,
) -> Option<u64> {
    let effective = source_bytes_for_file(config, cat, file, env).await?;
    desired_content_hash(file, &effective).ok()
}

pub fn desired_content_hash(file: &metadata::AppFile, bytes: &[u8]) -> Result<u64> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(hash_content(bytes)),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::managed_hash(bytes, managed_keys)
        }
    }
}

pub fn installed_content_hash(file: &metadata::AppFile, bytes: &[u8]) -> Result<Option<u64>> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(Some(hash_content(bytes))),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::installed_hash(bytes, managed_keys)
        }
    }
}

async fn install_prepared_content(
    file: &metadata::AppFile,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => {
            if file.requires_admin {
                file_ops::install_bytes_admin(content, destination, is_managed, dry_run, force)
                    .await
            } else {
                file_ops::install_bytes(content, destination, is_managed, dry_run, force).await
            }
        }
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::install(content, destination, dry_run, managed_keys).await
        }
    }
}

async fn uninstall_app_entry(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    match &entry.install_strategy {
        AppInstallStrategy::Copy if entry.requires_admin => {
            file_ops::uninstall_entry_admin(entry, dry_run, force).await
        }
        AppInstallStrategy::Copy => file_ops::uninstall_entry(entry, dry_run, force).await,
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::uninstall(entry, dry_run, force, managed_keys).await
        }
    }
}

fn app_category_from_source(source: &str) -> Option<String> {
    app_source_parts(source).map(|(category, _)| category.to_string())
}

fn app_source_parts(source: &str) -> Option<(&str, &str)> {
    let mut parts = source.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("app"), Some(category), Some(file)) => Some((category, file)),
        _ => None,
    }
}

pub fn resolve_install_destination(
    category: &metadata::AppCategory,
    file: &metadata::AppFile,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(dest_root) = &category.destination_root {
        let expanded = crate::config::full_expand_with_home(dest_root, &config.home_dir)
            .with_context(|| format!("failed to expand destination root: {dest_root}"))?;
        let root = PathBuf::from(&expanded);
        if !is_install_destination_root_absolute(&expanded, &root) {
            anyhow::bail!("destination root must be absolute after expansion");
        }
        if root
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            anyhow::bail!("destination root must not contain '..'");
        }
        return Ok(root.join(&file.target_rel));
    }

    annotation::resolve_destination(
        file.legacy_dest_annotation.as_deref(),
        &category.name,
        &file.target_rel.display().to_string(),
        config,
    )
}

#[cfg(windows)]
fn is_install_destination_root_absolute(_expanded: &str, root: &Path) -> bool {
    root.is_absolute()
}

#[cfg(not(windows))]
fn is_install_destination_root_absolute(expanded: &str, root: &Path) -> bool {
    root.is_absolute() || expanded.starts_with('/')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::config::Config;
    use crate::install_core::manifest::AppManifest;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-apps").await
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
        let mut manifest = "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n".to_string();
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
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\npost_upgrade = {{ command = \"/bin/sh\", args = [\"{}\", \"{}\"] }}\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n",
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
            utils::init_template::write_shine_toml_template(&cat_dir, false, APP_TEMPLATE).unwrap();
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

        let err =
            utils::init_template::write_shine_toml_template(&dir, false, APP_TEMPLATE).unwrap_err();
        assert!(
            err.to_string().contains("use --force to overwrite"),
            "unexpected error: {err:#}"
        );
        assert_eq!(fs::read(dir.join("shine.toml")).await.unwrap(), b"old");

        let (_path, overwritten) =
            utils::init_template::write_shine_toml_template(&dir, true, APP_TEMPLATE).unwrap();
        assert!(overwritten);
        let content = fs::read_to_string(dir.join("shine.toml")).await.unwrap();
        assert!(content.contains("dest = \"~/.config/my-app\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn install_resolves_windows_docker_engine_destination_on_windows() {
        let dir = std::env::temp_dir().join("shine-apps-win-dest");
        let config = Config::new_for_test(&dir);
        let categories = metadata::load_embedded_categories(Some("docker-engine")).unwrap();
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-engine")
            .unwrap();
        let file = docker.files.first().unwrap();

        let destination = resolve_install_destination(docker, file, &config).unwrap();

        assert_eq!(
            destination,
            PathBuf::from(crate::config::full_expand("~/.docker").unwrap()).join("daemon.json")
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_accepts_unix_metadata_destination_on_unix() {
        let dir = std::env::temp_dir().join("shine-apps-unix-dest");
        let config = Config::new_for_test(&dir);
        let categories = metadata::load_embedded_categories(Some("docker-engine")).unwrap();
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-engine")
            .unwrap();
        let file = docker.files.first().unwrap();

        let destination = resolve_install_destination(docker, file, &config).unwrap();

        assert_eq!(
            destination,
            PathBuf::from("/etc/docker").join("daemon.json")
        );
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0, "up-to-date app config must not update");
        assert_eq!(report.skipped, 1, "up-to-date app config should be skipped");
        assert_eq!(fs::read(&dest).await.unwrap(), before);

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();
        let manifest_before = AppManifest::load(config.shine_dir()).await.unwrap();
        let hash_before = manifest_before.entries[0].content_hash;

        write_external_sample_app(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
        )
        .await;
        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1, "changed source should update");
        assert_eq!(report.skipped, 0);
        assert_ne!(fs::read(&dest).await.unwrap(), before);
        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert_ne!(manifest_after.entries[0].content_hash, hash_before);

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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
        config.allow_app_hooks = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
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

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1);
        assert_eq!(fs::read_to_string(&marker).await.unwrap(), "x");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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
        config.allow_app_hooks = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0);
        assert!(!marker.exists(), "unchanged config must not run hook");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
            &script,
            &marker,
        )
        .await;

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1);
        assert!(
            !marker.exists(),
            "external hook must be skipped unless allow_app_hooks is enabled"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        write_external_sample_app_with_extra(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            Some(b"background = @@GHOSTTY_BG_LIGHT@@\n"),
        )
        .await;

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        let new_dest = dir.join(".config/sample/themes/theme.conf");
        assert_eq!(report.updated, 1, "new app file should be installed");
        assert_eq!(
            report.skipped, 1,
            "existing up-to-date file should be skipped"
        );
        assert_eq!(
            fs::read(&new_dest).await.unwrap(),
            b"background = \n",
            "new file should be transformed before install"
        );
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_some(),
            "new app file should be tracked in manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
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

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0, "unmanaged existing file must not update");
        assert_eq!(
            report.skipped, 2,
            "existing managed file and unmanaged new file should be skipped"
        );
        assert_eq!(fs::read(&new_dest).await.unwrap(), b"user-owned\n");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_none(),
            "unmanaged destination should not be added to manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 1, "stale cleanup should count as a change");
        assert_eq!(report.skipped, 0);
        assert!(!dest.exists(), "unmodified stale file should be removed");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "stale manifest entry should be removed"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_file(&dest).await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 1, "manifest cleanup should count as change");
        assert_eq!(report.skipped, 0);
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "missing stale destination should be removed from manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert!(dest.exists(), "stale file should be left in place");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "stale manifest entry should remain without prune"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.user_modified, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "user-modified stale entry should remain tracked"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let cat_dir = dir.join("presets/app/sample");
        fs::write(
            cat_dir.join("shine.toml"),
            b"description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon-renamed.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"jsonc-to-json\"]\n",
        )
        .await
        .unwrap();
        fs::write(
            cat_dir.join("daemon-renamed.jsonc"),
            b"{\n  \"proxy\": \"new\"\n}\n",
        )
        .await
        .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        let dest = dir.join(".config/sample/daemon.json");
        assert_eq!(
            report.updated, 2,
            "cleanup plus reinstall should change state"
        );
        assert_eq!(report.skipped, 0);
        assert_eq!(
            fs::read(&dest).await.unwrap(),
            b"{\n  \"proxy\": \"new\"\n}\n"
        );
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        let entry = manifest.find_by_dest(&dest).unwrap();
        assert_eq!(entry.source, "app/sample/daemon-renamed.jsonc");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
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

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(
            report.updated, 0,
            "user-modified app config must not update"
        );
        assert_eq!(report.skipped, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
