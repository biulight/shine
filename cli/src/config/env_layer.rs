//! Env variable layering: `[env]` table parsing, legacy `env.toml` migration,
//! and the `shine.env.toml` override files (global, overlay, project).

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::fs;

use super::discovery::ProjectConfig;
use super::{
    Config, DEFAULT_ENV_VARS, LEGACY_ENV_FILE, LEGACY_PROJECT_ENV_FILE,
    LEGACY_PROJECT_FILES_REMOVAL_VERSION, PROJECT_ENV_FILE,
};

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
    pub(super) async fn migrate_env(&mut self, config_has_env: bool) -> Result<()> {
        let mut needs_save = !config_has_env;

        let legacy_path = self.shine_dir().join(LEGACY_ENV_FILE);
        let legacy_env = read_env_file(&legacy_path).await?;
        let has_legacy = legacy_env.is_some();

        if let Some(overrides) = legacy_env {
            for (key, value) in overrides.values {
                if config_has_env {
                    if let std::collections::btree_map::Entry::Vacant(entry) = self.env.entry(key) {
                        entry.insert(value);
                        needs_save = true;
                    }
                } else {
                    self.env.insert(key, value);
                    needs_save = true;
                }
            }
        }

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

        if has_legacy {
            match fs::remove_file(&legacy_path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("failed to remove {}", legacy_path.display()));
                }
            }
        }

        Ok(())
    }

    pub(super) async fn apply_global_env_override(&mut self) -> Result<()> {
        let env_path = self.shine_dir().join(PROJECT_ENV_FILE);
        let Some(overrides) = read_env_file(&env_path).await? else {
            return Ok(());
        };
        self.apply_env_overrides(overrides);
        Ok(())
    }

    pub(super) async fn apply_overlay_env_override(&mut self) -> Result<()> {
        let Some(overlay_dir) = self.active_presets_overlay_dir() else {
            return Ok(());
        };
        let Some(overrides) = read_env_file(&overlay_dir.join(PROJECT_ENV_FILE)).await? else {
            return Ok(());
        };
        self.apply_env_overrides(overrides);
        Ok(())
    }

    pub(super) async fn apply_project_env_override(
        &mut self,
        project_config: &ProjectConfig,
    ) -> Result<()> {
        let env_path = project_config.root.join(PROJECT_ENV_FILE);
        let (env_path, is_legacy) = if env_path.exists() {
            (env_path, false)
        } else {
            let legacy_env_path = project_config.root.join(LEGACY_PROJECT_ENV_FILE);
            (legacy_env_path, true)
        };
        let Some(overrides) = read_env_file(&env_path).await? else {
            return Ok(());
        };

        if is_legacy {
            eprintln!(
                "Warning: project env {} is deprecated and will no longer be supported in {}; rename it to {}.",
                env_path.display(),
                LEGACY_PROJECT_FILES_REMOVAL_VERSION,
                project_config.root.join(PROJECT_ENV_FILE).display()
            );
        }

        self.apply_env_overrides(overrides);
        Ok(())
    }

    fn apply_env_overrides(&mut self, overrides: EnvOverrides) {
        self.env.extend(overrides.values);
        self.env_descriptions.extend(overrides.descriptions);
    }
}

#[doc(hidden)]
pub async fn validate_env_override_file(path: &Path) -> Result<()> {
    read_env_file(path).await.map(|_| ())
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
    async fn load_or_init_migrates_legacy_env_file_and_deletes_it() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("env.toml"),
            "HTTP_PROXY_PORT = \"7890\"\nCUSTOM_TOKEN = \"abc\"\n",
        )
        .await
        .unwrap();
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("7890")
        );
        assert_eq!(
            config.env.get("CUSTOM_TOKEN").map(String::as_str),
            Some("abc")
        );
        assert!(
            !dir.join("env.toml").exists(),
            "legacy env.toml should be removed"
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
    async fn config_env_wins_over_legacy_env_file() {
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
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("1111")
        );
        assert_eq!(
            config.env.get("CUSTOM_TOKEN").map(String::as_str),
            Some("abc")
        );
        assert!(
            !dir.join("env.toml").exists(),
            "legacy env.toml should be removed"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
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
                is_legacy: false,
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
}
