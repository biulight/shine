use crate::{config::Config, presets};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use tokio::fs;

const ENV_CATALOG_FILE: &str = "env.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvMetadata {
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

pub(crate) async fn load(config: &Config) -> Result<BTreeMap<String, EnvMetadata>> {
    let bytes = if config.is_external_presets {
        let path = config.presets_dir().join(ENV_CATALOG_FILE);
        match fs::read(&path).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        }
    } else {
        presets::read_asset_bytes(ENV_CATALOG_FILE)
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
