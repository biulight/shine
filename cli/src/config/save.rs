//! Config persistence: coordinated atomic writes and raw-layer delta saves.

use crate::output::escape_controls;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
#[cfg(test)]
use tokio::fs;

use super::Config;

impl Config {
    pub async fn save(&self) -> Result<()> {
        let config_path = self.resolve_config_path_for_save().await?;

        let _guard = super::env_write::lock_target(&config_path).await?;
        // Validate and parse the same regular-file preimage under the shared lock.
        let current = super::ConfigInput::capture(&config_path).await?;
        if let Some(input) = self
            .config_inputs
            .iter()
            .find(|input| input.path == config_path)
            && input != &current
        {
            bail!(
                "configuration changed at {}; retry the command",
                escape_controls(&config_path.display().to_string())
            );
        }
        let new_table = self.serialize_table_for_save()?;
        let toml_str = if let Some(existing) = current.contents.filter(|text| !text.is_empty()) {
            let mut doc: toml_edit::DocumentMut = existing
                .parse()
                .map_err(super::config_parse_error)
                .context("Failed to parse existing config for comment preservation")?;
            shine_core::migration::sync_table(doc.as_table_mut(), &new_table);
            doc.to_string()
        } else {
            toml::to_string_pretty(&new_table).context("Failed to serialize config")?
        };

        crate::persist::atomic_write_private(&config_path, toml_str.as_bytes())
            .await
            .with_context(|| format!("Failed to write config to {config_path:?}"))?;

        Ok(())
    }

    fn serialize_table_for_save(&self) -> Result<toml::Table> {
        let table = self.serialize_effective_table()?;
        let mut sparse = if let Some(state) = &self.save_state {
            let mut sparse = state.original.clone();
            apply_table_changes(&mut sparse, &state.loaded, &table);
            sparse
        } else {
            table
        };
        for (key, value) in &self.env_pending {
            let env = sparse
                .entry("env".to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let env = env.as_table_mut().context("[env] must be a table")?;
            match value {
                Some(value) => {
                    env.insert(key.clone(), toml::Value::String(value.clone()));
                }
                None => {
                    env.remove(key);
                }
            }
        }
        if self.is_project_config {
            sparse.remove("schema_version");
            sparse.remove("last_cleared_schema_version");
            // Executable sys-code permission is intentionally global-only. Never preserve or
            // materialize it in a project configuration that could authorize its own presets.
            sparse.remove("allow_app_hooks");
            sparse.remove("allow_sys_code");
            sparse.remove("hybrid_decrypt_backend");
            return Ok(sparse);
        }

        Ok(sparse)
    }

    pub(super) fn serialize_effective_table(&self) -> Result<toml::Table> {
        let serialized = toml::to_string_pretty(self).context("Failed to serialize config")?;
        toml::from_str(&serialized)
            .map_err(super::config_parse_error)
            .context("Failed to round-trip serialize config")
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

    async fn save_and_read(config: &Config) -> String {
        config.save().await.unwrap();
        fs::read_to_string(&config.config_path).await.unwrap()
    }

    async fn save_fresh_config(edit: impl FnOnce(&mut Config)) -> String {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        edit(&mut config);
        let content = save_and_read(&config).await;
        fs::remove_dir_all(dir).await.unwrap();
        content
    }

    #[tokio::test]
    async fn config_writers_discard_source_bearing_parse_error_chains() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let path = config.config_path();
        for contents in [
            "TOKEN = \"PRIVATE_CONFIG_CANARY_UNCLOSED\n",
            "PRIVATE_CONFIG_CANARY_KEY = 1\nPRIVATE_CONFIG_CANARY_KEY = 2\n",
        ] {
            fs::write(path, contents).await.unwrap();
            let errors = [
                config.save().await.unwrap_err(),
                crate::config::write_env_override_entry(path, "TOKEN", Some("new"))
                    .await
                    .unwrap_err(),
                crate::config::validate_env_override_file(path)
                    .await
                    .unwrap_err(),
            ];
            for error in errors {
                for diagnostic in [format!("{error:#}"), format!("{error:?}")] {
                    assert!(diagnostic.contains("TOML parse error"), "{diagnostic}");
                    assert!(
                        !diagnostic.contains("PRIVATE_CONFIG_CANARY"),
                        "{diagnostic}"
                    );
                    assert!(!diagnostic.contains("TOKEN ="), "{diagnostic}");
                }
            }
            assert_eq!(fs::read_to_string(path).await.unwrap(), contents);
        }
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn save_keeps_config_private_on_creation_and_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        for previous_mode in [None, Some(0o600), Some(0o644)] {
            if let Some(mode) = previous_mode {
                fs::set_permissions(&config.config_path, std::fs::Permissions::from_mode(mode))
                    .await
                    .unwrap();
            }
            config.save().await.unwrap();
            assert_eq!(
                fs::metadata(&config.config_path)
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_current_schema_to_missing_and_empty_files() {
        for existing in [false, true] {
            let dir = make_temp_dir().await;
            let config = config_in(&dir);
            if existing {
                fs::write(&config.config_path, b"").await.unwrap();
            }
            let content = save_and_read(&config).await;
            let parsed: toml::Table = toml::from_str(&content).unwrap();
            assert_eq!(
                parsed["schema_version"].as_integer(),
                Some(CURRENT_RUNTIME_SCHEMA_VERSION.into()),
                "existing file: {existing}"
            );
            fs::remove_dir_all(dir).await.unwrap();
        }
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
        let content = save_and_read(&updated).await;
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_integer(), Some(2));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_last_cleared_schema_version_when_set() {
        let content = save_fresh_config(|config| {
            config.last_cleared_schema_version = Some(1);
        })
        .await;
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["last_cleared_schema_version"].as_integer(), Some(1));
    }

    #[tokio::test]
    async fn save_merges_preserves_comments() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.schema_version = 0;
        fs::write(&config.config_path, "# keep this\nschema_version = 0\n")
            .await
            .unwrap();

        let content = save_and_read(&config).await;
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

        let content = save_and_read(&config).await;
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

        let content = save_and_read(&config).await;
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
        let content = save_fresh_config(|config| {
            config.presets_dir_override = Some(PathBuf::from("/external/presets"));
        })
        .await;
        assert!(
            content.contains("/external/presets"),
            "presets_dir should be written to config.toml"
        );

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_dir_override,
            Some(PathBuf::from("/external/presets"))
        );
    }

    #[tokio::test]
    async fn presets_overlay_dir_override_round_trips_through_save() {
        let content = save_fresh_config(|config| {
            config.presets_overlay_dir_override = Some(PathBuf::from("/external/overlay"));
        })
        .await;
        assert!(content.contains("presets_overlay_dir"));

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_overlay_dir_override,
            Some(PathBuf::from("/external/overlay"))
        );
    }

    #[tokio::test]
    async fn presets_overlay_git_round_trips_through_save() {
        let content = save_fresh_config(|config| {
            config.presets_overlay_git = Some("https://example.com/overlay.git".to_string());
            config.presets_overlay_git_branch = Some("main".to_string());
        })
        .await;
        assert!(content.contains("presets_overlay_git"));

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_overlay_git.as_deref(),
            Some("https://example.com/overlay.git")
        );
        assert_eq!(loaded.presets_overlay_git_branch.as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn gpg_recipients_round_trip_through_save() {
        let content = save_fresh_config(|config| {
            config.gpg_recipients = vec![
                "alice@example.com".to_string(),
                "bob@example.com".to_string(),
            ];
        })
        .await;
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.gpg_recipients,
            ["alice@example.com", "bob@example.com"]
        );
    }

    #[tokio::test]
    async fn retired_coarse_trust_flags_are_not_saved() {
        let content = save_fresh_config(|config| {
            config.legacy_allow_app_hooks = true;
            config.legacy_allow_sys_code = true;
        })
        .await;
        assert!(!content.contains("allow_app_hooks"));
        assert!(!content.contains("allow_sys_code"));
    }

    #[tokio::test]
    async fn sync_terminal_theme_false_round_trips_through_save() {
        let content = save_fresh_config(|config| {
            config.sync_terminal_theme = false;
        })
        .await;
        assert!(content.contains("sync_terminal_theme"));
        let loaded: Config = toml::from_str(&content).unwrap();
        assert!(!loaded.sync_terminal_theme);
    }

    #[tokio::test]
    async fn fresh_config_omits_default_theme_and_presets_dir() {
        let content = save_fresh_config(|_| {}).await;
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            !parsed.contains_key("sync_terminal_theme"),
            "default true value must not clutter a fresh config.toml"
        );
        assert!(
            !parsed.contains_key("presets_dir"),
            "presets_dir key must be absent when override is None"
        );
        // An old config.toml with no such key at all must still default true.
        let loaded: Config = toml::from_str(&content).unwrap();
        assert!(loaded.sync_terminal_theme);
    }
}
