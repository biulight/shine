//! Config persistence: atomic writes, comment preservation, and sparse
//! project-layer diffing so inherited global values never materialize in
//! project config files.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use tokio::fs;

use super::Config;

impl Config {
    pub async fn save(&self) -> Result<()> {
        let config_path = self.resolve_config_path_for_save().await?;

        let new_table = self.serialize_table_for_save()?;
        let new_toml = toml::to_string_pretty(&new_table).context("Failed to serialize config")?;

        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("Fail to parse existing config for comment preservation")?;

                utils::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        crate::persist::atomic_write(&config_path, toml_str.as_bytes())
            .await
            .with_context(|| format!("Failed to write config to {config_path:?}"))?;

        Ok(())
    }

    fn serialize_table_for_save(&self) -> Result<toml::Table> {
        let table = self.serialize_effective_table()?;

        if self.is_project_config {
            let mut sparse = if let Some(state) = &self.project_save_state {
                let mut sparse = state.original.clone();
                apply_table_changes(&mut sparse, &state.loaded, &table);
                sparse
            } else {
                table
            };
            sparse.remove("schema_version");
            sparse.remove("last_cleared_schema_version");
            return Ok(sparse);
        }

        Ok(table)
    }

    pub(super) fn serialize_effective_table(&self) -> Result<toml::Table> {
        let serialized = toml::to_string_pretty(self).context("Failed to serialize config")?;
        toml::from_str(&serialized).context("Failed to round-trip serialize config")
    }

    async fn resolve_config_path_for_save(&self) -> Result<PathBuf> {
        if self
            .config_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
        {
            return Ok(self.config_path.clone());
        }
        bail!("config path must not be empty");
    }
}

fn apply_table_changes(target: &mut toml::Table, loaded: &toml::Table, current: &toml::Table) {
    let keys: std::collections::BTreeSet<_> = loaded.keys().chain(current.keys()).collect();
    for key in keys {
        match (loaded.get(key), current.get(key)) {
            (Some(toml::Value::Table(before)), Some(toml::Value::Table(after))) => {
                let entry = target
                    .entry(key.clone())
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if !entry.is_table() {
                    *entry = toml::Value::Table(toml::Table::new());
                }
                apply_table_changes(entry.as_table_mut().unwrap(), before, after);
                if entry.as_table().is_some_and(toml::Table::is_empty) {
                    target.remove(key);
                }
            }
            (before, after) if before == after => {}
            (_, Some(value)) => {
                target.insert(key.clone(), value.clone());
            }
            (_, None) => {
                target.remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{config_in, make_temp_dir};
    use super::*;
    use crate::config::CURRENT_RUNTIME_SCHEMA_VERSION;
    use std::path::Path;

    #[tokio::test]
    async fn save_writes_config_file_for_new_config() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_new_toml_when_existing_file_is_empty() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(&config.config_path, b"").await.unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(!content.is_empty());
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(parsed.contains_key("schema_version"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_updates_changed_value() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(&config.config_path, "schema_version = 0\n")
            .await
            .unwrap();

        let updated = Config {
            schema_version: 2,
            ..config
        };
        updated.save().await.unwrap();

        let content = fs::read_to_string(&updated.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_integer(), Some(2));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_last_cleared_schema_version_when_set() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.last_cleared_schema_version = Some(1);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["last_cleared_schema_version"].as_integer(), Some(1));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_preserves_comments() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.schema_version = 0;
        fs::write(&config.config_path, "# keep this\nschema_version = 0\n")
            .await
            .unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("# keep this"),
            "comment should be preserved"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_updates_detailed_env_value_without_losing_description() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        fs::write(
            &config.config_path,
            "[env]\nMY_TOKEN = { value = \"old\", description = \"Internal token\" }\n",
        )
        .await
        .unwrap();
        config.env.insert("MY_TOKEN".into(), "new".into());

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("MY_TOKEN = { value = \"new\", description = \"Internal token\" }")
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_removes_stale_keys() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(
            &config.config_path,
            "schema_version = 0\nstale_key = \"old\"\n",
        )
        .await
        .unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            !content.contains("stale_key"),
            "stale key should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_returns_error_for_path_without_parent() {
        let config = Config {
            config_path: PathBuf::from("config.toml"),
            ..Config::new_for_test(Path::new("shine"))
        };
        assert!(config.save().await.is_err());
    }

    #[tokio::test]
    async fn presets_dir_override_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.presets_dir_override = Some(PathBuf::from("/external/presets"));

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("/external/presets"),
            "presets_dir should be written to config.toml"
        );

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_dir_override,
            Some(PathBuf::from("/external/presets"))
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn presets_overlay_dir_override_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(PathBuf::from("/external/overlay"));

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(content.contains("presets_overlay_dir"));

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_overlay_dir_override,
            Some(PathBuf::from("/external/overlay"))
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn gpg_key_id_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("alice@example.com".to_string());

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(loaded.gpg_key_id.as_deref(), Some("alice@example.com"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn allow_app_hooks_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.allow_app_hooks = true;

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert!(loaded.allow_app_hooks);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn presets_dir_absent_from_toml_when_override_is_none() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir); // presets_dir_override: None

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            !parsed.contains_key("presets_dir"),
            "presets_dir key must be absent when override is None"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
