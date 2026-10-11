//! One deterministic resolver for every env consumer. Raw layers never serialize.
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::fs;

use super::{Config, EnvOverrideKind, EnvOverrideSource, default_env_map};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnvLayerKind {
    Builtin,
    Overlay,
    GlobalConfig,
    GlobalEnv,
    ProjectConfig,
    ProjectEnv,
}

impl EnvLayerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Overlay => "overlay",
            Self::GlobalConfig => "global config",
            Self::GlobalEnv => "global env file",
            Self::ProjectConfig => "project config",
            Self::ProjectEnv => "project env file",
        }
    }
    pub fn is_config(self) -> bool {
        matches!(self, Self::GlobalConfig | Self::ProjectConfig)
    }
}

#[derive(Clone, Debug)]
pub struct EnvLayer {
    pub kind: EnvLayerKind,
    pub path: Option<PathBuf>,
    pub values: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
}

/// Private preimage; never include file contents in diagnostics or public DTOs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigInput {
    pub path: PathBuf,
    pub contents: Option<String>,
}

impl ConfigInput {
    pub async fn capture(path: &Path) -> Result<Self> {
        // Reject special files before opening, including dangling symlinks.
        match fs::symlink_metadata(path).await {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => bail!(
                "{} must be a regular file, not a symlink or special file",
                path.display()
            ),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("checking {}", path.display())),
        }
        let read_path = path.to_path_buf();
        let contents = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            use std::io::Read;
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                // Inspect the link itself if an editor swaps it after metadata checking.
                options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
            }
            let mut file = match options.open(&read_path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => {
                    return Err(e).with_context(|| format!("opening {}", read_path.display()));
                }
            };
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                bail!("{} must be a regular file", read_path.display());
            }
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .with_context(|| format!("reading {}", read_path.display()))?;
            Ok(Some(contents))
        })
        .await??;
        Ok(Self {
            path: path.to_path_buf(),
            contents,
        })
    }

    pub async fn verify(&self) -> Result<()> {
        if Self::capture(&self.path).await? != *self {
            bail!(
                "configuration changed at {}; retry the command",
                self.path.display()
            );
        }
        Ok(())
    }
}

impl Config {
    pub(super) async fn resolve_env_layers(&mut self) -> Result<()> {
        self.reject_removed_global_env_file().await?;
        let mut layers = vec![EnvLayer {
            kind: EnvLayerKind::Builtin,
            path: None,
            values: default_env_map(),
            descriptions: BTreeMap::new(),
        }];
        let mut inputs = vec![ConfigInput::capture(&self.shine_dir().join("env.toml")).await?];
        let mut paths = Vec::new();
        if let Some(overlay) = self.active_presets_overlay_dir() {
            paths.push((EnvLayerKind::Overlay, overlay.join(super::PROJECT_ENV_FILE)));
        }
        paths.push((EnvLayerKind::GlobalConfig, self.global_config_path()));
        paths.push((
            EnvLayerKind::GlobalEnv,
            self.shine_dir().join(super::PROJECT_ENV_FILE),
        ));
        if self.is_project_config() {
            paths.push((
                EnvLayerKind::ProjectConfig,
                self.config_path().to_path_buf(),
            ));
            paths.push((
                EnvLayerKind::ProjectEnv,
                self.config_path().with_file_name(super::PROJECT_ENV_FILE),
            ));
        }
        for (kind, path) in paths {
            let input = ConfigInput::capture(&path).await?;
            if let Some(before) = self.config_inputs.iter().find(|i| i.path == path)
                && before != &input
            {
                bail!(
                    "configuration changed at {}; retry the command",
                    path.display()
                );
            }
            let mut values = BTreeMap::new();
            let mut descriptions = BTreeMap::new();
            if let Some(contents) = &input.contents {
                let table: toml::Table = toml::from_str(contents)
                    .with_context(|| format!("parsing {}", path.display()))?;
                let entries = if kind.is_config() {
                    match table.get("env") {
                        Some(value) => value.as_table().context("[env] must be a table")?.clone(),
                        None => toml::Table::new(),
                    }
                } else {
                    table
                };
                for (key, value) in entries {
                    let parsed: super::env_layer::EnvValue =
                        value.try_into().with_context(|| {
                            format!("invalid env variable `{key}` in {}", path.display())
                        })?;
                    match parsed {
                        super::env_layer::EnvValue::Plain(value) => {
                            values.insert(key, value);
                        }
                        super::env_layer::EnvValue::Detailed { value, description } => {
                            values.insert(key.clone(), value);
                            if let Some(description) = description {
                                descriptions.insert(key, description);
                            }
                        }
                    }
                }
            }
            inputs.push(input);
            layers.push(EnvLayer {
                kind,
                path: Some(path),
                values,
                descriptions,
            });
        }
        self.env.clear();
        self.env_descriptions.clear();
        self.env_override_sources.clear();
        self.env_description_sources.clear();
        for layer in &layers {
            let kind = match layer.kind {
                EnvLayerKind::Builtin => EnvOverrideKind::Builtin,
                EnvLayerKind::Overlay => EnvOverrideKind::Overlay,
                EnvLayerKind::GlobalConfig => EnvOverrideKind::GlobalConfig,
                EnvLayerKind::GlobalEnv => EnvOverrideKind::Global,
                EnvLayerKind::ProjectConfig => EnvOverrideKind::ProjectConfig,
                EnvLayerKind::ProjectEnv => EnvOverrideKind::Project,
            };
            for key in layer.values.keys() {
                self.env_override_sources.insert(
                    key.clone(),
                    EnvOverrideSource {
                        path: layer.path.clone().unwrap_or_default(),
                        kind,
                        is_managed_overlay: layer.kind == EnvLayerKind::Overlay
                            && self.presets_overlay_dir_override.is_none(),
                    },
                );
            }
            for key in layer.descriptions.keys() {
                self.env_description_sources.insert(key.clone(), layer.kind);
            }
            self.env.extend(layer.values.clone());
            self.env_descriptions.extend(layer.descriptions.clone());
        }
        self.env_layers = layers;
        self.config_inputs = inputs;
        Ok(())
    }

    pub fn env_value_layer(&self, key: &str) -> Option<&EnvLayer> {
        self.env_layers
            .iter()
            .rev()
            .find(|layer| layer.values.contains_key(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ConfigSaveState, EnvWriteOptions, EnvWritePlan},
        env::EnvConfig,
    };

    async fn loaded_config(dir: &Path) -> Config {
        let mut config = Config::new_for_test(dir);
        let original: toml::Table =
            toml::from_str(&fs::read_to_string(config.config_path()).await.unwrap()).unwrap();
        config.presets_overlay_dir_override = Some(dir.join("overlay"));
        config.resolve_env_layers().await.unwrap();
        let loaded = config.serialize_effective_table().unwrap();
        config.save_state = Some(ConfigSaveState { original, loaded });
        config
    }

    #[tokio::test]
    async fn unrelated_save_keeps_raw_env_and_explicit_equal_default_is_saved() {
        let dir = crate::test_support::make_temp_dir("env-save").await;
        fs::create_dir_all(dir.join("overlay")).await.unwrap();
        fs::write(dir.join("config.toml"), "# retained\n[env]\nHISTORICAL = '6152'\nCUSTOM = { value = 'local', description = 'keep' }\n").await.unwrap();
        fs::write(dir.join("overlay/shine.env.toml"), "BASELINE = 'overlay'\n")
            .await
            .unwrap();
        fs::write(dir.join("shine.env.toml"), "SIDECAR = 'sidecar'\n")
            .await
            .unwrap();
        let mut config = loaded_config(&dir).await;
        config.secret_backend = Some("age".into());
        config.save().await.unwrap();
        let saved = fs::read_to_string(config.config_path()).await.unwrap();
        assert!(saved.contains("# retained"));
        assert!(saved.contains("description = 'keep'"));
        let table: toml::Table = toml::from_str(&saved).unwrap();
        assert_eq!(table["env"].as_table().unwrap().len(), 2);
        assert_eq!(table["env"]["HISTORICAL"].as_str(), Some("6152"));
        let config = loaded_config(&dir).await;
        let mut env = EnvConfig::from_config(&config);
        env.set("HTTP_PROXY_PORT", "6152");
        env.save(&config).await.unwrap();
        let config = loaded_config(&dir).await;
        assert_eq!(
            config.env_value_layer("HTTP_PROXY_PORT").unwrap().kind,
            EnvLayerKind::GlobalConfig
        );
        assert_eq!(
            config.env_value_layer("SIDECAR").unwrap().kind,
            EnvLayerKind::GlobalEnv
        );
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn config_save_and_env_commit_share_snapshot_rejection() {
        let dir = crate::test_support::make_temp_dir("env-race").await;
        fs::write(dir.join("config.toml"), "[env]\nORIGINAL = 'retained'\n")
            .await
            .unwrap();
        let mut config = loaded_config(&dir).await;
        let options = EnvWriteOptions {
            global: true,
            ..Default::default()
        };
        let one = EnvWritePlan::prepare(&config, "ONE", &options, false)
            .await
            .unwrap();
        let two = EnvWritePlan::prepare(&config, "TWO", &options, false)
            .await
            .unwrap();
        one.commit(&config, "ONE", Some("new")).await.unwrap();
        let error = two.commit(&config, "TWO", Some("stale")).await.unwrap_err();
        assert!(error.to_string().contains("retry"));
        config.secret_backend = Some("age".into());
        assert!(
            config
                .save()
                .await
                .unwrap_err()
                .to_string()
                .contains("retry")
        );
        let contents = fs::read_to_string(config.config_path()).await.unwrap();
        assert!(contents.contains("ONE"));
        assert!(!contents.contains("TWO"));
        assert!(contents.contains("ORIGINAL"));
        assert!(!contents.contains("secret_backend"));
        fs::remove_dir_all(dir).await.unwrap();
    }
}
