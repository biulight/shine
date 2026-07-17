pub mod catalog;
pub mod commands;
pub mod identity;
pub mod upgrade;
pub mod workspace;

use crate::config::Config;
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};

/// User-editable environment variables stored in `config.toml` under `[env]`.
///
/// Values are substituted into preset files that opt in via the `template`
/// transform (using `@@VAR_NAME@@` placeholders).
#[derive(Clone, Debug, Default)]
pub struct EnvConfig {
    vars: BTreeMap<String, String>,
    descriptions: BTreeMap<String, String>,
}

impl EnvConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            vars: config.env.clone(),
            descriptions: config.env_descriptions.clone(),
        }
    }

    pub async fn load_or_init(config: &Config) -> Result<Self> {
        Ok(Self::from_config(config))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.vars.remove(key)
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn description(&self, key: &str) -> Option<&str> {
        self.descriptions.get(key).map(String::as_str)
    }

    pub async fn save(&self, config: &Config) -> Result<()> {
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

#[derive(Debug, PartialEq, Eq)]
/// A config environment value selected using encrypted-first lookup.
pub enum StoredValue<'a> {
    Secret { key: String, value: &'a str },
    Plaintext(&'a str),
}

/// Resolve `KEY_SECRET` first, falling back to plaintext `KEY`.
pub fn resolve_stored_value<'a>(env: &'a EnvConfig, key: &str) -> Result<StoredValue<'a>> {
    let secret_key = secret_key(key);
    if let Some(value) = env.get(&secret_key) {
        return Ok(StoredValue::Secret {
            key: secret_key,
            value,
        });
    }
    if let Some(value) = env.get(key) {
        return Ok(StoredValue::Plaintext(value));
    }
    bail!("{secret_key} or {key} is not set in the active config [env]");
}

/// Return the encrypted-storage key associated with an environment variable.
pub fn secret_key(key: &str) -> String {
    format!("{key}_SECRET")
}

/// A validated environment declaration using the `env run --with` grammar:
/// resolve `source` from the active config and expose it under `target`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVarSpec {
    pub source: String,
    pub target: String,
}

impl EnvVarSpec {
    /// The `--with` argument token that reproduces this declaration:
    /// `KEY` when source and target match, otherwise `SOURCE=TARGET`.
    pub fn to_with_arg(&self) -> String {
        if self.source == self.target {
            self.source.clone()
        } else {
            format!("{}={}", self.source, self.target)
        }
    }
}

/// Parse and validate an ordered list of `KEY` / `SOURCE_KEY=TARGET_KEY` specs
/// (the shared grammar for `env run --with` and a Bun preset's `env` array).
///
/// Declaration order is preserved. Both names must be valid environment
/// identifiers, and no two declarations may write the same target. Values are
/// never resolved here — this is pure name validation, safe to run at metadata
/// load time.
pub(crate) fn parse_env_specs(specs: &[String]) -> Result<Vec<EnvVarSpec>> {
    let mut parsed = Vec::with_capacity(specs.len());
    let mut targets = BTreeSet::new();
    for spec in specs {
        let (source, target) = spec.split_once('=').unwrap_or((spec, spec));
        validate_env_key(source)?;
        validate_env_key(target)?;
        if !targets.insert(target.to_string()) {
            bail!("duplicate target variable: {target}");
        }
        parsed.push(EnvVarSpec {
            source: source.to_string(),
            target: target.to_string(),
        });
    }
    Ok(parsed)
}

/// Validate an environment variable name: first character `[A-Za-z_]`, remaining
/// characters `[A-Za-z0-9_]*`.
pub(crate) fn validate_env_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("environment variable name must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        bail!("invalid environment variable name: {key}");
    }
    Ok(())
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
