use crate::{config::Config, presets};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use tokio::fs;

const ENV_CATALOG_FILE: &str = "env.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvMetadata {
    pub description: String,
    pub sensitive: bool,
}

#[derive(Deserialize)]
struct EnvCatalogToml {
    #[serde(default)]
    variables: Vec<EnvMetadataToml>,
}

#[derive(Deserialize)]
struct EnvMetadataToml {
    key: String,
    description: String,
    #[serde(default)]
    sensitive: bool,
}

pub async fn load(config: &Config) -> Result<BTreeMap<String, EnvMetadata>> {
    let bytes = if config.is_external_presets {
        let path = config.preset_path(ENV_CATALOG_FILE);
        match fs::read(&path).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        }
    } else {
        presets::PresetAssetReader::new(config.active_presets_overlay_dir())
            .read_asset_bytes(ENV_CATALOG_FILE)
    };

    bytes
        .as_deref()
        .map(parse)
        .transpose()
        .map(Option::unwrap_or_default)
}

fn parse(bytes: &[u8]) -> Result<BTreeMap<String, EnvMetadata>> {
    let parsed: EnvCatalogToml = toml::from_slice(bytes).context("failed to parse env.toml")?;
    let mut catalog = BTreeMap::new();
    for variable in parsed.variables {
        if variable.key.trim().is_empty() {
            bail!("env.toml contains an empty variable key");
        }
        if catalog
            .insert(
                variable.key.clone(),
                EnvMetadata {
                    description: variable.description,
                    sensitive: variable.sensitive,
                },
            )
            .is_some()
        {
            bail!(
                "env.toml contains duplicate variable key `{}`",
                variable.key
            );
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalogs_follow_each_config_including_replaced_overlay_and_managed_availability() {
        let root = crate::test_support::make_temp_dir("shine-catalog").await;
        let a = root.join("a");
        let b = root.join("b");
        for (dir, key) in [(&a, "ONLY_A"), (&b, "ONLY_B")] {
            fs::create_dir_all(dir).await.unwrap();
            fs::write(
                dir.join("env.toml"),
                format!(
                    "[[variables]]\nkey = \"{key}\"\ndescription = \"overlay\"\nsensitive = true\n"
                ),
            )
            .await
            .unwrap();
        }
        let base = Config::new_for_test(&root);
        let config_a = base
            .clone()
            .with_presets_overlay_dir_override(Some(a.clone()));
        let config_b = config_a.clone().with_presets_overlay_dir_override(Some(b));
        let builtin = parse(&presets::read_embedded_asset_bytes("env.toml").unwrap()).unwrap();
        for _ in 0..2 {
            assert_eq!(load(&base).await.unwrap(), builtin);
            for (config, key) in [(&config_a, "ONLY_A"), (&config_b, "ONLY_B")] {
                let catalog = load(config).await.unwrap();
                assert_eq!(catalog.len(), 1);
                assert!(catalog[key].sensitive);
            }
        }
        let managed =
            base.with_presets_overlay_git(Some("https://example.invalid/overlay.git".into()), None);
        assert_eq!(load(&managed).await.unwrap(), builtin);
        fs::rename(&a, root.join("overlay")).await.unwrap();
        assert!(load(&managed).await.unwrap().contains_key("ONLY_A"));
        let manual = managed.with_presets_overlay_dir_override(
            config_b.active_presets_overlay_dir().map(ToOwned::to_owned),
        );
        assert!(load(&manual).await.unwrap().contains_key("ONLY_B"));
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn external_catalog_remains_authoritative_and_invalid_overlay_fails() {
        let root = crate::test_support::make_temp_dir("shine-catalog-external").await;
        let mut config = Config::new_for_test(&root);
        config.is_external_presets = true;
        assert!(load(&config).await.unwrap().is_empty());
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::write(
            config.presets_dir().join("env.toml"),
            "[[variables]]\nkey = \"EXTERNAL\"\ndescription = \"external\"\n",
        )
        .await
        .unwrap();
        assert!(load(&config).await.unwrap().contains_key("EXTERNAL"));
        let overlay = root.join("overlay");
        fs::create_dir_all(&overlay).await.unwrap();
        fs::write(overlay.join("env.toml"), "invalid TOML [")
            .await
            .unwrap();
        let config = config.with_presets_overlay_dir_override(Some(overlay));
        assert!(
            load(&config)
                .await
                .unwrap_err()
                .to_string()
                .contains("failed to parse env.toml")
        );
        let mut builtin = config;
        builtin.is_external_presets = false;
        assert!(
            load(&builtin)
                .await
                .unwrap_err()
                .to_string()
                .contains("failed to parse env.toml")
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn parses_catalog_metadata() {
        let catalog = parse(
            br#"
                [[variables]]
                key = "API_TOKEN"
                description = "API access token"
                sensitive = true
            "#,
        )
        .unwrap();

        assert_eq!(catalog["API_TOKEN"].description, "API access token");
        assert!(catalog["API_TOKEN"].sensitive);
    }

    #[test]
    fn rejects_duplicate_keys() {
        let error = parse(
            br#"
                [[variables]]
                key = "PORT"
                description = "First"
                [[variables]]
                key = "PORT"
                description = "Second"
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("duplicate variable key `PORT`"));
    }
}
