//! Env variable layering: `[env]` defaults and parsing, removed global
//! `env.toml` detection, and `shine.env.toml` override files.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::fs;

use super::discovery::ProjectConfig;
use super::{Config, DEFAULT_ENV_VARS, EnvOverrideKind, EnvOverrideSource, PROJECT_ENV_FILE};

const REMOVED_GLOBAL_ENV_FILE: &str = "env.toml";

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum EnvValue {
    Plain(String),
    Detailed {
        value: String,
        description: Option<String>,
    },
}

#[derive(Default)]
struct EnvOverrides {
    values: BTreeMap<String, String>,
    descriptions: BTreeMap<String, String>,
}

pub(super) fn deserialize_env_values<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    let values = BTreeMap::<String, EnvValue>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                EnvValue::Plain(value) | EnvValue::Detailed { value, .. } => value,
            };
            (key, value)
        })
        .collect())
}

pub(super) fn parse_env_descriptions(contents: &str) -> BTreeMap<String, String> {
    let Ok(table) = toml::from_str::<toml::Table>(contents) else {
        return BTreeMap::new();
    };
    let Some(env) = table.get("env").and_then(toml::Value::as_table) else {
        return BTreeMap::new();
    };
    env.iter()
        .filter_map(|(key, value)| {
            value
                .as_table()
                .and_then(|entry| entry.get("description"))
                .and_then(toml::Value::as_str)
                .map(|description| (key.clone(), description.to_string()))
        })
        .collect()
}

impl Config {
    pub(super) async fn ensure_env_defaults(&mut self, config_has_env: bool) -> Result<()> {
        self.reject_removed_global_env_file().await?;
        let mut needs_save = !config_has_env;

        for (key, value) in DEFAULT_ENV_VARS {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.env.entry(key.to_string())
            {
                entry.insert(value.to_string());
                needs_save = true;
            }
        }

        if needs_save {
            self.save().await?;
        }

        Ok(())
    }

    async fn reject_removed_global_env_file(&self) -> Result<()> {
        let removed_path = self.shine_dir().join(REMOVED_GLOBAL_ENV_FILE);
        if !fs::try_exists(&removed_path)
            .await
            .with_context(|| format!("checking {}", removed_path.display()))?
        {
            return Ok(());
        }

        let override_path = self.shine_dir().join(PROJECT_ENV_FILE);
        if fs::try_exists(&override_path)
            .await
            .with_context(|| format!("checking {}", override_path.display()))?
        {
            bail!(
                "the removed global env file {} still exists; run a v0.39 shine binary once to \
                 migrate it, or manually merge its values into {} and then remove the old file",
                removed_path.display(),
                override_path.display()
            );
        }

        bail!(
            "the removed global env file {} still exists; run a v0.39 shine binary once to \
             migrate it, or rename it to {}",
            removed_path.display(),
            override_path.display()
        )
    }

    pub(super) async fn apply_global_env_override(&mut self) -> Result<()> {
        let env_path = self.shine_dir().join(PROJECT_ENV_FILE);
        let Some(overrides) = read_env_file(&env_path).await? else {
            return Ok(());
        };
        self.apply_env_overrides(env_path, EnvOverrideKind::Global, false, overrides);
        Ok(())
    }

    pub(super) async fn apply_overlay_env_override(&mut self) -> Result<()> {
        let Some(overlay_dir) = self.active_presets_overlay_dir() else {
            return Ok(());
        };
        let env_path = overlay_dir.join(PROJECT_ENV_FILE);
        let Some(overrides) = read_env_file(&env_path).await? else {
            return Ok(());
        };
        // If no manual overlay dir is configured, `active_presets_overlay_dir()`
        // can only have resolved to the shine-managed Git checkout.
        let is_managed_overlay = self.presets_overlay_dir_override.is_none();
        self.apply_env_overrides(
            env_path,
            EnvOverrideKind::Overlay,
            is_managed_overlay,
            overrides,
        );
        Ok(())
    }

    pub(super) async fn apply_project_env_override(
        &mut self,
        project_config: &ProjectConfig,
    ) -> Result<()> {
        let env_path = project_config.root.join(PROJECT_ENV_FILE);
        let Some(overrides) = read_env_file(&env_path).await? else {
            return Ok(());
        };

        self.apply_env_overrides(env_path, EnvOverrideKind::Project, false, overrides);
        Ok(())
    }

    fn apply_env_overrides(
        &mut self,
        path: std::path::PathBuf,
        kind: EnvOverrideKind,
        is_managed_overlay: bool,
        overrides: EnvOverrides,
    ) {
        for key in overrides.values.keys() {
            self.env_override_sources.insert(
                key.clone(),
                EnvOverrideSource {
                    path: path.clone(),
                    kind,
                    is_managed_overlay,
                },
            );
        }
        self.env.extend(overrides.values);
        self.env_descriptions.extend(overrides.descriptions);
    }
}

#[doc(hidden)]
pub async fn validate_env_override_file(path: &Path) -> Result<()> {
    read_env_file(path).await.map(|_| ())
}

/// Set (`Some`) or remove (`None`) one key in a `shine.env.toml`-shaped override
/// file, preserving comments/formatting of unrelated entries and the
/// `description` of a `{ value, description }` entry being updated. Creates the
/// file (and its parent directory) if it doesn't exist yet.
pub async fn write_env_override_entry(path: &Path, key: &str, value: Option<&str>) -> Result<()> {
    let existing = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut target: toml::Table = if existing.is_empty() {
        toml::Table::new()
    } else {
        toml::from_str(&existing).with_context(|| format!("parsing {}", path.display()))?
    };

    match value {
        // Always target a plain string, mirroring `Config::env`'s always-flat
        // `BTreeMap<String, String>` shape. `sync_table` itself detects when the
        // *document* still has the detailed `{ value, description }` form and
        // updates only the `value` sub-field, preserving `description` — the
        // same mechanism `Config::save()` relies on for config.toml `[env]`.
        Some(value) => {
            target.insert(key.to_string(), toml::Value::String(value.to_string()));
        }
        None => {
            target.remove(key);
        }
    }

    let new_content = if existing.is_empty() {
        toml::to_string_pretty(&target).context("serializing env override file")?
    } else {
        let mut doc: toml_edit::DocumentMut = existing
            .parse()
            .context("parsing existing env override file for comment preservation")?;
        shine_core::migration::sync_table(doc.as_table_mut(), &target);
        doc.to_string()
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    crate::persist::atomic_write(path, new_content.as_bytes())
        .await
        .with_context(|| format!("writing {}", path.display()))
}

async fn read_env_file(path: &Path) -> Result<Option<EnvOverrides>> {
    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    let table: toml::Table =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let mut overrides = EnvOverrides::default();
    for (key, value) in table {
        let parsed: EnvValue = value.try_into().with_context(|| {
            format!(
                "invalid env variable `{key}` in {}: expected a string or {{ value, description }}",
                path.display()
            )
        })?;
        match parsed {
            EnvValue::Plain(value) => {
                overrides.values.insert(key, value);
            }
            EnvValue::Detailed { value, description } => {
                overrides.values.insert(key.clone(), value);
                if let Some(description) = description {
                    overrides.descriptions.insert(key, description);
                }
            }
        }
    }
    Ok(Some(overrides))
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{config_in, make_temp_dir, restore_current_dir};
    use super::*;
    use crate::config::PROJECT_CONFIG_FILE;
    use crate::test_support::env_lock;

    #[test]
    fn detailed_env_values_are_normalized_with_descriptions() {
        let contents = r#"
            [env]
            PLAIN = "value"
            TOKEN = { value = "secret", description = "Internal token" }
        "#;
        let config: Config = toml::from_str(contents).unwrap();

        assert_eq!(config.env.get("PLAIN").map(String::as_str), Some("value"));
        assert_eq!(config.env.get("TOKEN").map(String::as_str), Some("secret"));
        assert_eq!(
            parse_env_descriptions(contents)
                .get("TOKEN")
                .map(String::as_str),
            Some("Internal token")
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_rejects_removed_global_env_file_without_mutating_state() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(dir.join("env.toml"), "not valid = [\n")
            .await
            .unwrap();
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let error = Config::load_or_init().await.unwrap_err().to_string();

        assert!(error.contains(&dir.join("env.toml").display().to_string()));
        assert!(error.contains("v0.39"));
        assert!(error.contains("rename"));
        assert!(error.contains(&dir.join("shine.env.toml").display().to_string()));
        assert!(
            dir.join("env.toml").exists(),
            "removed env.toml must be left for explicit recovery"
        );
        assert!(
            !dir.join("config.toml").exists(),
            "rejection must happen before config initialization"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_reads_global_shine_env_without_presets_dir() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("config.toml"),
            "[env]\nHTTP_PROXY_PORT = \"1111\"\nCONFIG_ONLY = \"config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nSIDECAR_ONLY = \"sidecar\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("2222")
        );
        assert_eq!(
            config.env.get("CONFIG_ONLY").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("SIDECAR_ONLY").map(String::as_str),
            Some("sidecar")
        );
        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        assert!(
            !content.contains("SIDECAR_ONLY"),
            "global shine.env.toml overrides should not be written into config.toml"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn project_shine_env_overrides_global_shine_env() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        fs::create_dir_all(&child_dir).await.unwrap();
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\n[env]\nHTTP_PROXY_PORT = \"1111\"\nCONFIG_ONLY = \"config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            state_dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nGLOBAL_ONLY = \"global\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"3333\"\nPROJECT_ONLY = \"project\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("3333")
        );
        assert_eq!(
            config.env.get("CONFIG_ONLY").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("GLOBAL_ONLY").map(String::as_str),
            Some("global")
        );
        assert_eq!(
            config.env.get("PROJECT_ONLY").map(String::as_str),
            Some("project")
        );

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn removed_global_env_file_requires_manual_merge_when_override_exists() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("config.toml"),
            "[env]\nHTTP_PROXY_PORT = \"1111\"\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nCUSTOM_TOKEN = \"abc\"\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"3333\"\nSIDECAR_ONLY = \"sidecar\"\n",
        )
        .await
        .unwrap();
        let original_config = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let original_removed = fs::read_to_string(dir.join("env.toml")).await.unwrap();
        let original_override = fs::read_to_string(dir.join("shine.env.toml"))
            .await
            .unwrap();
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let error = Config::load_or_init().await.unwrap_err().to_string();

        assert!(error.contains("v0.39"));
        assert!(error.contains("manually merge"));
        assert!(error.contains(&dir.join("env.toml").display().to_string()));
        assert!(error.contains(&dir.join("shine.env.toml").display().to_string()));
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).await.unwrap(),
            original_config
        );
        assert_eq!(
            fs::read_to_string(dir.join("env.toml")).await.unwrap(),
            original_removed
        );
        assert_eq!(
            fs::read_to_string(dir.join("shine.env.toml"))
                .await
                .unwrap(),
            original_override
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn project_dotenv_toml_is_ignored() {
        let dir = make_temp_dir().await;
        let project_dir = dir.join("project");
        fs::create_dir_all(&project_dir).await.unwrap();
        fs::write(
            project_dir.join(".env.toml"),
            "GENERIC_DOTENV_VALUE = \"ignored\"\n",
        )
        .await
        .unwrap();
        let mut config = config_in(&dir);

        config
            .apply_project_env_override(&ProjectConfig {
                path: project_dir.join(PROJECT_CONFIG_FILE),
                root: project_dir,
            })
            .await
            .unwrap();

        assert!(!config.env.contains_key("GENERIC_DOTENV_VALUE"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_env_sits_between_global_and_project_env_overrides() {
        let dir = make_temp_dir().await;
        let overlay_dir = dir.join("overlay");
        let project_dir = dir.join("project");
        fs::create_dir_all(&overlay_dir).await.unwrap();
        fs::create_dir_all(&project_dir).await.unwrap();
        fs::write(
            dir.join(PROJECT_ENV_FILE),
            "SHARED = { value = \"global\", description = \"global description\" }\n\
             DETAILED = { value = \"global\", description = \"global detail\" }\n",
        )
        .await
        .unwrap();
        fs::write(
            overlay_dir.join(PROJECT_ENV_FILE),
            "SHARED = { value = \"overlay\", description = \"overlay description\" }\n\
             DETAILED = { value = \"overlay\", description = \"overlay detail\" }\n\
             OVERLAY_ONLY = { value = \"yes\", description = \"overlay only\" }\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join(PROJECT_ENV_FILE),
            "SHARED = \"project\"\n\
             DETAILED = { value = \"project\", description = \"project detail\" }\n",
        )
        .await
        .unwrap();

        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(overlay_dir);
        config.apply_global_env_override().await.unwrap();
        config.apply_overlay_env_override().await.unwrap();
        assert_eq!(
            config.env.get("SHARED").map(String::as_str),
            Some("overlay")
        );
        assert_eq!(
            config.env.get("OVERLAY_ONLY").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            config.env_descriptions.get("SHARED").map(String::as_str),
            Some("overlay description")
        );
        assert_eq!(
            config
                .env_descriptions
                .get("OVERLAY_ONLY")
                .map(String::as_str),
            Some("overlay only")
        );

        config
            .apply_project_env_override(&ProjectConfig {
                path: project_dir.join(PROJECT_CONFIG_FILE),
                root: project_dir,
            })
            .await
            .unwrap();
        assert_eq!(
            config.env.get("SHARED").map(String::as_str),
            Some("project")
        );
        assert_eq!(
            config.env_descriptions.get("SHARED").map(String::as_str),
            Some("overlay description"),
            "a plain override must preserve the inherited description"
        );
        assert_eq!(
            config.env.get("DETAILED").map(String::as_str),
            Some("project")
        );
        assert_eq!(
            config.env_descriptions.get("DETAILED").map(String::as_str),
            Some("project detail")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_override_rejects_invalid_values_with_path_and_key() {
        let dir = make_temp_dir().await;
        let path = dir.join(PROJECT_ENV_FILE);
        let invalid_values = [
            "TOKEN = 42\n",
            "TOKEN = [\"one\", \"two\"]\n",
            "TOKEN = { description = \"missing value\" }\n",
            "TOKEN = { value = 42, description = \"wrong type\" }\n",
        ];

        for content in invalid_values {
            fs::write(&path, content).await.unwrap();
            let error = validate_env_override_file(&path).await.unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("TOKEN"), "missing key in: {message}");
            assert!(
                message.contains(path.to_str().unwrap()),
                "missing path in: {message}"
            );
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_env_is_loaded_when_external_presets_are_active() {
        let dir = make_temp_dir().await;
        let overlay_dir = dir.join("overlay");
        fs::create_dir_all(&overlay_dir).await.unwrap();
        fs::write(
            overlay_dir.join(PROJECT_ENV_FILE),
            "OVERLAY_ONLY = \"yes\"\n",
        )
        .await
        .unwrap();

        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(overlay_dir);
        config.is_external_presets = true;
        config.apply_overlay_env_override().await.unwrap();
        assert_eq!(
            config.env.get("OVERLAY_ONLY").map(String::as_str),
            Some("yes")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn global_override_records_key_source() {
        let dir = make_temp_dir().await;
        fs::write(dir.join(PROJECT_ENV_FILE), "TOKEN = \"global\"\n")
            .await
            .unwrap();

        let mut config = config_in(&dir);
        config.apply_global_env_override().await.unwrap();

        let source = config.env_override_source("TOKEN").unwrap();
        assert_eq!(source.path, dir.join(PROJECT_ENV_FILE));
        assert_eq!(source.kind, EnvOverrideKind::Global);
        assert!(!source.is_managed_overlay);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn manual_overlay_override_records_unmanaged_source() {
        let dir = make_temp_dir().await;
        let overlay_dir = dir.join("overlay");
        fs::create_dir_all(&overlay_dir).await.unwrap();
        fs::write(overlay_dir.join(PROJECT_ENV_FILE), "TOKEN = \"overlay\"\n")
            .await
            .unwrap();

        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(overlay_dir.clone());
        config.apply_overlay_env_override().await.unwrap();

        let source = config.env_override_source("TOKEN").unwrap();
        assert_eq!(source.path, overlay_dir.join(PROJECT_ENV_FILE));
        assert_eq!(source.kind, EnvOverrideKind::Overlay);
        assert!(
            !source.is_managed_overlay,
            "a manual overlay dir must not be reported as the managed mirror"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn managed_git_overlay_override_records_managed_source() {
        let dir = make_temp_dir().await;
        let overlay_dir = dir.join("overlay");
        fs::create_dir_all(&overlay_dir).await.unwrap();
        fs::write(overlay_dir.join(PROJECT_ENV_FILE), "TOKEN = \"overlay\"\n")
            .await
            .unwrap();

        let mut config = config_in(&dir);
        // No manual override configured; the managed checkout existing on disk
        // is what `active_presets_overlay_dir()` falls back to.
        config.managed_overlay_dir = Some(overlay_dir.clone());
        config.apply_overlay_env_override().await.unwrap();

        let source = config.env_override_source("TOKEN").unwrap();
        assert_eq!(source.path, overlay_dir.join(PROJECT_ENV_FILE));
        assert_eq!(source.kind, EnvOverrideKind::Overlay);
        assert!(
            source.is_managed_overlay,
            "the shine-managed Git overlay mirror must be flagged as managed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn later_override_layer_wins_the_recorded_source_for_shared_key() {
        let dir = make_temp_dir().await;
        let overlay_dir = dir.join("overlay");
        let project_dir = dir.join("project");
        fs::create_dir_all(&overlay_dir).await.unwrap();
        fs::create_dir_all(&project_dir).await.unwrap();
        fs::write(dir.join(PROJECT_ENV_FILE), "SHARED = \"global\"\n")
            .await
            .unwrap();
        fs::write(overlay_dir.join(PROJECT_ENV_FILE), "SHARED = \"overlay\"\n")
            .await
            .unwrap();
        fs::write(project_dir.join(PROJECT_ENV_FILE), "SHARED = \"project\"\n")
            .await
            .unwrap();

        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(overlay_dir.clone());
        config.apply_global_env_override().await.unwrap();
        config.apply_overlay_env_override().await.unwrap();
        assert_eq!(
            config.env_override_source("SHARED").unwrap().path,
            overlay_dir.join(PROJECT_ENV_FILE),
            "overlay layer must overwrite the global layer's recorded source"
        );

        config
            .apply_project_env_override(&ProjectConfig {
                path: project_dir.join(PROJECT_CONFIG_FILE),
                root: project_dir.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            config.env_override_source("SHARED").unwrap().path,
            project_dir.join(PROJECT_ENV_FILE),
            "project layer must overwrite the overlay layer's recorded source"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn config_toml_only_key_has_no_override_source() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.env.insert("PLAIN".into(), "value".into());
        config.apply_global_env_override().await.unwrap();
        config.apply_overlay_env_override().await.unwrap();

        assert!(config.env_override_source("PLAIN").is_none());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn write_env_override_entry_inserts_new_plain_key() {
        let dir = make_temp_dir().await;
        let path = dir.join(PROJECT_ENV_FILE);

        write_env_override_entry(&path, "TOKEN", Some("secret"))
            .await
            .unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        let table: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            table.get("TOKEN").and_then(toml::Value::as_str),
            Some("secret")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn write_env_override_entry_updates_existing_plain_key_and_keeps_comments() {
        let dir = make_temp_dir().await;
        let path = dir.join(PROJECT_ENV_FILE);
        // The comment guards an *unchanged* key: sync_table only preserves
        // formatting/comments for entries it doesn't have to touch.
        fs::write(
            &path,
            "TOKEN = \"old\"\n# keep this comment\nOTHER = \"unchanged\"\n",
        )
        .await
        .unwrap();

        write_env_override_entry(&path, "TOKEN", Some("new"))
            .await
            .unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("# keep this comment"));
        assert!(content.contains("TOKEN = \"new\""));
        assert!(content.contains("OTHER = \"unchanged\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn write_env_override_entry_updates_value_and_preserves_description() {
        let dir = make_temp_dir().await;
        let path = dir.join(PROJECT_ENV_FILE);
        fs::write(
            &path,
            "TOKEN = { value = \"old\", description = \"Internal token\" }\n",
        )
        .await
        .unwrap();

        write_env_override_entry(&path, "TOKEN", Some("new"))
            .await
            .unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("TOKEN = { value = \"new\", description = \"Internal token\" }"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn write_env_override_entry_removes_key() {
        let dir = make_temp_dir().await;
        let path = dir.join(PROJECT_ENV_FILE);
        fs::write(&path, "TOKEN = \"secret\"\nOTHER = \"unchanged\"\n")
            .await
            .unwrap();

        write_env_override_entry(&path, "TOKEN", None)
            .await
            .unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        let table: toml::Table = toml::from_str(&content).unwrap();
        assert!(!table.contains_key("TOKEN"));
        assert_eq!(
            table.get("OTHER").and_then(toml::Value::as_str),
            Some("unchanged")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn write_env_override_entry_creates_missing_file_and_parent_dir() {
        let dir = make_temp_dir().await;
        let path = dir.join("nested").join(PROJECT_ENV_FILE);

        write_env_override_entry(&path, "TOKEN", Some("secret"))
            .await
            .unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        let table: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            table.get("TOKEN").and_then(toml::Value::as_str),
            Some("secret")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
