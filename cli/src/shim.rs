//! Top-level `shine install/reinstall/uninstall <category>` shims: they infer
//! whether `category` names a shell preset or an app preset (or ask the user
//! when both match) and delegate to the corresponding handler.

use anyhow::{Result, bail};
use dialoguer::Select;

use crate::config::Config;
use crate::{apps, shells};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PresetKind {
    Shell,
    App,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ShimResolution {
    Found(PresetKind),
    Conflict,
    Missing,
}

pub async fn handle_install_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_install(config, Some(category), false)).await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_install(config, Some(category), false, false)).await
        }
        ShimResolution::Conflict => match select_shim_kind("Install", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_install(config, Some(category), false)).await
            }
            PresetKind::App => {
                Box::pin(apps::handle_install(config, Some(category), false, false)).await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

pub async fn handle_reinstall_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_install(config, Some(category), true)).await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_install(config, Some(category), false, true)).await
        }
        ShimResolution::Conflict => match select_shim_kind("Reinstall", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_install(config, Some(category), true)).await
            }
            PresetKind::App => {
                Box::pin(apps::handle_install(config, Some(category), false, true)).await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

pub async fn handle_uninstall_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_uninstall(
                config,
                Some(category),
                false,
                false,
            ))
            .await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_uninstall(
                config,
                Some(category),
                false,
                false,
                false,
            ))
            .await
        }
        ShimResolution::Conflict => match select_shim_kind("Uninstall", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_uninstall(
                    config,
                    Some(category),
                    false,
                    false,
                ))
                .await
            }
            PresetKind::App => {
                Box::pin(apps::handle_uninstall(
                    config,
                    Some(category),
                    false,
                    false,
                    false,
                ))
                .await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

async fn resolve_shim_category(config: &Config, category: &str) -> Result<ShimResolution> {
    // Not migrated to metadata::load_active_categories: this guards with an
    // existence check before calling load_installed_categories in external
    // mode, deliberately returning 0 matches instead of propagating that
    // function's `bail!` on an empty result — load_active_categories would
    // change resolve_shim_category's error semantics here.
    let shell_matches = if config.is_external_presets {
        let shell_path = config.preset_path(std::path::Path::new("shell").join(category));
        if shell_path.exists() {
            shells::metadata::load_installed_categories(config, Some(category))
                .await?
                .len()
        } else {
            0
        }
    } else {
        shells::metadata::load_embedded_categories(Some(category))?.len()
    };
    let app_matches = if config.is_external_presets {
        let app_path = config.preset_path(std::path::Path::new("app").join(category));
        if app_path.exists() {
            apps::load_installed_categories(config, Some(category))
                .await?
                .len()
        } else {
            0
        }
    } else {
        apps::load_embedded_categories(Some(category))?.len()
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

fn select_shim_kind(action: &str, category: &str) -> Result<PresetKind> {
    let choices = [format!("shell/{category}"), format!("app/{category}")];
    let selected = Select::new()
        .with_prompt(format!("{action} which preset?"))
        .items(&choices)
        .default(0)
        .interact()?;
    Ok(match selected {
        0 => PresetKind::Shell,
        1 => PresetKind::App,
        _ => unreachable!("dialoguer Select returned out-of-range index {selected}"),
    })
}

fn bail_shim_missing(category: &str) -> Result<()> {
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
}
