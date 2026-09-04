//! Top-level `shine install/uninstall <target>` shims: they resolve canonical
//! or unambiguous preset targets and delegate to the corresponding handler.

use anyhow::{Result, bail};

use crate::config::Config;
use crate::{apps, shells};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum PresetKind {
    Shell,
    App,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ShimResolution {
    Found(PresetKind),
    Conflict,
    Missing,
}

pub async fn handle_install_shim(
    config: &Config,
    target: &str,
    replace_managed: bool,
) -> Result<()> {
    handle_install_shim_approved(config, target, replace_managed, true).await
}

pub async fn handle_install_shim_approved(
    config: &Config,
    target: &str,
    replace_managed: bool,
    yes: bool,
) -> Result<()> {
    let (explicit_kind, category) = parse_preset_target(target)?;
    if explicit_kind == Some(PresetKind::Shell) && category.contains('/') {
        return Box::pin(shells::handle_install_approved(
            config,
            Some(category),
            replace_managed,
            yes,
        ))
        .await;
    }
    match resolve_shim_target(config, explicit_kind, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_install_approved(
                config,
                Some(category),
                replace_managed,
                yes,
            ))
            .await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_install_approved(
                config,
                Some(category),
                false,
                replace_managed,
                yes,
            ))
            .await
        }
        ShimResolution::Conflict => bail_ambiguous(category),
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

pub async fn handle_uninstall_shim(
    config: &Config,
    target: &str,
    force: bool,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    handle_uninstall_shim_approved(config, target, force, purge, dry_run, true).await
}

pub async fn handle_uninstall_shim_approved(
    config: &Config,
    target: &str,
    force: bool,
    purge: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let (explicit_kind, category) = parse_preset_target(target)?;
    if explicit_kind == Some(PresetKind::Shell) && category.contains('/') {
        if force {
            bail!("`--force` applies only to app presets");
        }
        return Box::pin(shells::handle_uninstall_approved(
            config,
            Some(category),
            purge,
            dry_run,
            yes,
        ))
        .await;
    }
    match resolve_shim_target(config, explicit_kind, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            if force {
                bail!("`--force` applies only to app presets");
            }
            Box::pin(shells::handle_uninstall_approved(
                config,
                Some(category),
                purge,
                dry_run,
                yes,
            ))
            .await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_uninstall_approved(
                config,
                Some(category),
                force,
                purge,
                dry_run,
                yes,
            ))
            .await
        }
        ShimResolution::Conflict => bail_ambiguous(category),
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

pub(crate) async fn resolve_preset_kind(
    config: &Config,
    target: &str,
) -> Result<(PresetKind, String)> {
    let (explicit_kind, target) = parse_preset_target(target)?;
    let category = if explicit_kind == Some(PresetKind::Shell) {
        target.split('/').next().unwrap_or_default()
    } else {
        target
    };
    match resolve_shim_target(config, explicit_kind, category).await? {
        ShimResolution::Found(kind) => Ok((kind, category.to_string())),
        ShimResolution::Conflict => bail_ambiguous(category),
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

fn parse_preset_target(target: &str) -> Result<(Option<PresetKind>, &str)> {
    let target = target.trim();
    if target.is_empty() {
        bail!("preset target must not be empty");
    }
    let (kind, category) = match target.split_once('/') {
        Some(("app", category)) => (Some(PresetKind::App), category),
        Some(("shell", category)) => (Some(PresetKind::Shell), category),
        Some((kind, _)) => bail!(
            "unsupported preset target kind `{kind}`; expected app/<category> or shell/<category>[/<command>]"
        ),
        None => (None, target),
    };
    let valid = match kind {
        Some(PresetKind::Shell) => {
            let mut parts = category.split('/');
            parts.next().is_some_and(|part| !part.is_empty())
                && parts.next().is_none_or(|part| !part.is_empty())
                && parts.next().is_none()
        }
        Some(PresetKind::App) | None => !category.is_empty() && !category.contains('/'),
    };
    if !valid {
        bail!(
            "invalid preset target `{target}`; expected app/<category>, shell/<category>[/<command>], or a unique category name"
        );
    }
    Ok((kind, category))
}

async fn resolve_shim_target(
    config: &Config,
    explicit_kind: Option<PresetKind>,
    category: &str,
) -> Result<ShimResolution> {
    if let Some(kind) = explicit_kind {
        let resolution = resolve_shim_category(config, category).await?;
        return Ok(match (kind, resolution) {
            (
                PresetKind::Shell,
                ShimResolution::Found(PresetKind::Shell) | ShimResolution::Conflict,
            ) => ShimResolution::Found(PresetKind::Shell),
            (
                PresetKind::App,
                ShimResolution::Found(PresetKind::App) | ShimResolution::Conflict,
            ) => ShimResolution::Found(PresetKind::App),
            _ => ShimResolution::Missing,
        });
    }
    resolve_shim_category(config, category).await
}

async fn resolve_shim_category(config: &Config, category: &str) -> Result<ShimResolution> {
    // Keep the external-source existence guards so a miss remains 0 matches
    // instead of propagating the active loader's not-found error. The loader
    // must still use the active snapshot: built-in presets can have an overlay
    // with categories that do not exist in the embedded namespace.
    let shell_path = config.preset_path(std::path::Path::new("shell").join(category));
    let shell_matches = if config.is_external_presets && !shell_path.exists() {
        0
    } else {
        shells::metadata::load_active_categories(config, Some(category))
            .await?
            .len()
    };
    let app_path = config.preset_path(std::path::Path::new("app").join(category));
    let app_matches = if config.is_external_presets && !app_path.exists() {
        0
    } else {
        apps::load_active_categories(config, Some(category))
            .await?
            .len()
    };

    Ok(classify_shim_resolution(shell_matches > 0, app_matches > 0))
}

fn classify_shim_resolution(shell_matches: bool, app_matches: bool) -> ShimResolution {
    match (shell_matches, app_matches) {
        (true, false) => ShimResolution::Found(PresetKind::Shell),
        (false, true) => ShimResolution::Found(PresetKind::App),
        (true, true) => ShimResolution::Conflict,
        (false, false) => ShimResolution::Missing,
    }
}

fn bail_ambiguous<T>(category: &str) -> Result<T> {
    bail!("ambiguous preset target `{category}`; use `app/{category}` or `shell/{category}`")
}

fn bail_shim_missing<T>(category: &str) -> Result<T> {
    bail!(
        "preset category not found in shell or app presets: {category}\nRun `shine shell list` or `shine app list` to see available categories."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-shim-test").await
    }

    fn config_in(dir: &std::path::Path) -> Config {
        crate::test_support::test_config(dir)
    }

    #[test]
    fn classify_shim_resolution_handles_all_match_shapes() {
        assert_eq!(
            classify_shim_resolution(true, false),
            ShimResolution::Found(PresetKind::Shell)
        );
        assert_eq!(
            classify_shim_resolution(false, true),
            ShimResolution::Found(PresetKind::App)
        );
        assert_eq!(
            classify_shim_resolution(true, true),
            ShimResolution::Conflict
        );
        assert_eq!(
            classify_shim_resolution(false, false),
            ShimResolution::Missing
        );
    }

    #[test]
    fn parse_preset_target_accepts_canonical_and_unique_shorthand_forms() {
        assert_eq!(
            parse_preset_target("app/starship").unwrap(),
            (Some(PresetKind::App), "starship")
        );
        assert_eq!(
            parse_preset_target("shell/proxy").unwrap(),
            (Some(PresetKind::Shell), "proxy")
        );
        assert_eq!(
            parse_preset_target("shell/utils/shine-env-export").unwrap(),
            (Some(PresetKind::Shell), "utils/shine-env-export")
        );
        assert_eq!(parse_preset_target("proxy").unwrap(), (None, "proxy"));
        assert!(parse_preset_target("sys/split-dns").is_err());
        assert!(parse_preset_target("app/surge/file").is_err());
        assert!(parse_preset_target("shell/utils/tool/extra").is_err());
    }

    #[tokio::test]
    async fn resolve_shim_category_matches_embedded_shell_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "proxy").await.unwrap();

        assert_eq!(resolution, ShimResolution::Found(PresetKind::Shell));
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_shim_category_matches_embedded_app_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "starship").await.unwrap();

        assert_eq!(resolution, ShimResolution::Found(PresetKind::App));
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_shim_category_reports_missing_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "does-not-exist")
            .await
            .unwrap();

        assert_eq!(resolution, ShimResolution::Missing);
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_shim_category_matches_overlay_only_categories_with_embedded_base() {
        let dir = make_temp_dir().await;
        let overlay = dir.join("overlay");
        fs::create_dir_all(overlay.join("shell/custom-shell"))
            .await
            .unwrap();
        fs::write(
            overlay.join("shell/custom-shell/shine.toml"),
            "description = 'Overlay Shell'\n[[files]]\nsource = 'custom.sh'\ntarget = 'custom'\n[files.permissions]\nschema_version = 1\n",
        )
        .await
        .unwrap();
        fs::write(overlay.join("shell/custom-shell/custom.sh"), "#!/bin/sh\n")
            .await
            .unwrap();
        fs::create_dir_all(overlay.join("app/custom-app"))
            .await
            .unwrap();
        fs::write(
            overlay.join("app/custom-app/shine.toml"),
            "metadata_schema_version = 2\ndescription = 'Overlay App'\ndest = '~/.config/custom-app'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\ntarget = 'config.toml'\n",
        )
        .await
        .unwrap();
        fs::write(
            overlay.join("app/custom-app/config.toml"),
            "enabled = true\n",
        )
        .await
        .unwrap();
        let config = config_in(&dir).with_presets_overlay_dir_override(Some(overlay));

        assert_eq!(
            resolve_shim_category(&config, "custom-shell")
                .await
                .unwrap(),
            ShimResolution::Found(PresetKind::Shell)
        );
        assert_eq!(
            resolve_shim_category(&config, "custom-app").await.unwrap(),
            ShimResolution::Found(PresetKind::App)
        );

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn canonical_shell_command_target_installs_and_uninstalls_one_command() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install_shim(&config, "shell/utils/shine-env-export", false)
            .await
            .unwrap();
        let selected = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        assert!(selected.exists());
        assert!(!sibling.exists());

        handle_uninstall_shim(&config, "shell/utils/shine-env-export", false, false, false)
            .await
            .unwrap();
        assert!(!selected.exists());

        fs::remove_dir_all(dir).await.unwrap();
    }
}
