pub(crate) mod upgrade;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

const ENV_FILE: &str = "env.toml";

/// Default variable values seeded into a new env.toml.
const DEFAULTS: &[(&str, &str)] = &[
    ("HTTP_PROXY_PORT", "6152"),
    ("SOCKS5_PROXY_PORT", "6153"),
    ("PROXY_HOST", "127.0.0.1"),
    ("PROXY_NO_PROXY", "localhost,127.0.0.1,::1"),
];

/// User-editable environment variables stored in `~/.shine/env.toml`.
///
/// Values are substituted into preset files that opt in via the `template`
/// transform (using `@@VAR_NAME@@` placeholders).
#[derive(Clone, Debug, Default)]
pub(crate) struct EnvConfig {
    vars: BTreeMap<String, String>,
}

impl EnvConfig {
    /// Load from `shine_dir/env.toml`, seeding defaults if the file is absent.
    /// Never overwrites existing keys with defaults.
    pub(crate) async fn load_or_init(shine_dir: &Path) -> Result<Self> {
        let path = shine_dir.join(ENV_FILE);

        if path.exists() {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?;
            let table: toml::Table =
                toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
            let mut vars: BTreeMap<String, String> = table
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect();

            // Backfill any DEFAULTS keys missing from the existing file so that
            // users who upgrade get new variables (e.g. PROXY_HOST) without
            // having to manually edit their env.toml.
            let mut needs_save = false;
            for (k, v) in DEFAULTS {
                if !vars.contains_key(*k) {
                    vars.insert(k.to_string(), v.to_string());
                    needs_save = true;
                }
            }
            let config = Self { vars };
            if needs_save {
                config.save(shine_dir).await?;
            }
            Ok(config)
        } else {
            let config = Self {
                vars: DEFAULTS
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            };
            config.save(shine_dir).await?;
            Ok(config)
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    pub(crate) fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub(crate) fn as_map(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Persist to `shine_dir/env.toml`, preserving any comments in an existing file
    /// using the same comment-preserving sync strategy as `Config::save`.
    pub(crate) async fn save(&self, shine_dir: &Path) -> Result<()> {
        let path = shine_dir.join(ENV_FILE);

        let new_toml = self.to_toml_string();

        let final_content = if path.exists() {
            let existing = fs::read_to_string(&path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let new_table: toml::Table = toml::from_str(&new_toml)
                    .context("failed to round-trip serialize env config")?;
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("failed to parse existing env.toml for comment preservation")?;
                utils::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        let temp = shine_dir.join(format!(".env-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .await
            .context("failed to create temp env file")?;
        file.write_all(final_content.as_bytes())
            .await
            .context("failed to write env file")?;
        file.sync_all().await.context("failed to sync env file")?;
        drop(file);

        fs::rename(&temp, &path)
            .await
            .context("failed to finalize env.toml")?;
        Ok(())
    }

    fn to_toml_string(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.vars {
            let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("{k} = \"{escaped}\"\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-env-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn load_or_init_creates_file_with_defaults() {
        let dir = make_temp_dir().await;
        let env = EnvConfig::load_or_init(&dir).await.unwrap();

        assert_eq!(env.get("HTTP_PROXY_PORT"), Some("6152"));
        assert_eq!(env.get("SOCKS5_PROXY_PORT"), Some("6153"));
        assert_eq!(env.get("PROXY_HOST"), Some("127.0.0.1"));
        assert_eq!(env.get("PROXY_NO_PROXY"), Some("localhost,127.0.0.1,::1"));
        assert!(dir.join("env.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_or_init_backfills_missing_defaults() {
        let dir = make_temp_dir().await;
        // Write a file that only has the old keys (no PROXY_HOST / PROXY_NO_PROXY).
        fs::write(
            dir.join("env.toml"),
            "HTTP_PROXY_PORT = \"7890\"\nSOCKS5_PROXY_PORT = \"7891\"\n",
        )
        .await
        .unwrap();

        let env = EnvConfig::load_or_init(&dir).await.unwrap();

        // Original values preserved.
        assert_eq!(env.get("HTTP_PROXY_PORT"), Some("7890"));
        assert_eq!(env.get("SOCKS5_PROXY_PORT"), Some("7891"));
        // Missing defaults backfilled.
        assert_eq!(env.get("PROXY_HOST"), Some("127.0.0.1"));
        assert_eq!(env.get("PROXY_NO_PROXY"), Some("localhost,127.0.0.1,::1"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_and_reload_roundtrip() {
        let dir = make_temp_dir().await;
        let mut env = EnvConfig::load_or_init(&dir).await.unwrap();
        env.set("HTTP_PROXY_PORT", "7890");
        env.save(&dir).await.unwrap();

        let reloaded = EnvConfig::load_or_init(&dir).await.unwrap();
        assert_eq!(reloaded.get("HTTP_PROXY_PORT"), Some("7890"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_does_not_overwrite_existing_file() {
        let dir = make_temp_dir().await;
        // Write a file with a custom port
        fs::write(dir.join("env.toml"), "HTTP_PROXY_PORT = \"9999\"\n")
            .await
            .unwrap();

        let env = EnvConfig::load_or_init(&dir).await.unwrap();
        assert_eq!(env.get("HTTP_PROXY_PORT"), Some("9999"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn set_and_get_roundtrip() {
        let mut env = EnvConfig::default();
        env.set("MY_VAR", "hello");
        assert_eq!(env.get("MY_VAR"), Some("hello"));
        assert_eq!(env.get("OTHER"), None);
    }

    #[test]
    fn as_map_reflects_all_vars() {
        let mut env = EnvConfig::default();
        env.set("A", "1");
        env.set("B", "2");
        let map = env.as_map();
        assert_eq!(map.get("A").map(|s| s.as_str()), Some("1"));
        assert_eq!(map.get("B").map(|s| s.as_str()), Some("2"));
    }
}
