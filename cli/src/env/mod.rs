pub(crate) mod catalog;
pub(crate) mod upgrade;
pub(crate) mod workspace;

use crate::config::Config;
use anyhow::Result;
use std::collections::BTreeMap;

/// User-editable environment variables stored in `config.toml` under `[env]`.
///
/// Values are substituted into preset files that opt in via the `template`
/// transform (using `@@VAR_NAME@@` placeholders).
#[derive(Clone, Debug, Default)]
pub(crate) struct EnvConfig {
    vars: BTreeMap<String, String>,
    descriptions: BTreeMap<String, String>,
}

impl EnvConfig {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            vars: config.env.clone(),
            descriptions: config.env_descriptions.clone(),
        }
    }

    pub(crate) async fn load_or_init(config: &Config) -> Result<Self> {
        Ok(Self::from_config(config))
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    pub(crate) fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<String> {
        self.vars.remove(key)
    }

    pub(crate) fn as_map(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub(crate) fn description(&self, key: &str) -> Option<&str> {
        self.descriptions.get(key).map(String::as_str)
    }

    pub(crate) async fn save(&self, config: &Config) -> Result<()> {
        let mut updated = config.clone();
        updated.env = self.vars.clone();
        updated.save().await
    }
}

impl From<EnvConfig> for BTreeMap<String, String> {
    fn from(value: EnvConfig) -> Self {
        value.vars
    }
}

impl EnvConfig {
    #[cfg(test)]
    fn with_defaults() -> Self {
        Self {
            vars: crate::config::default_env_map(),
            descriptions: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_reads_env_table() {
        let dir = std::env::temp_dir().join(format!("shine-env-test-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.env.insert("HTTP_PROXY_PORT".into(), "7890".into());

        let env = EnvConfig::from_config(&config);

        assert_eq!(env.get("HTTP_PROXY_PORT"), Some("7890"));
    }

    #[test]
    fn from_config_reads_description() {
        let dir = std::env::temp_dir().join(format!("shine-env-test-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config
            .env_descriptions
            .insert("MY_TOKEN".into(), "Internal token".into());

        let env = EnvConfig::from_config(&config);

        assert_eq!(env.description("MY_TOKEN"), Some("Internal token"));
    }

    #[test]
    fn set_and_get_roundtrip() {
        let mut env = EnvConfig::default();
        env.set("MY_VAR", "hello");
        assert_eq!(env.get("MY_VAR"), Some("hello"));
        assert_eq!(env.get("OTHER"), None);
    }

    #[test]
    fn remove_deletes_existing_key() {
        let mut env = EnvConfig::default();
        env.set("MY_VAR", "hello");

        assert_eq!(env.remove("MY_VAR"), Some("hello".to_string()));
        assert_eq!(env.get("MY_VAR"), None);
    }

    #[test]
    fn remove_missing_key_returns_none() {
        let mut env = EnvConfig::default();

        assert_eq!(env.remove("OTHER"), None);
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

    #[test]
    fn defaults_are_available_for_tests() {
        let env = EnvConfig::with_defaults();
        assert_eq!(env.get("HTTP_PROXY_PORT"), Some("6152"));
        assert_eq!(env.get("SOCKS5_PROXY_PORT"), Some("6153"));
        assert_eq!(env.get("PROXY_HOST"), Some("127.0.0.1"));
        assert_eq!(env.get("PROXY_NO_PROXY"), Some("localhost,127.0.0.1,::1"));
        assert_eq!(env.get("GHOSTTY_BG_LIGHT"), Some(""));
        assert_eq!(env.get("GHOSTTY_BG_DARK"), Some(""));
    }
}
