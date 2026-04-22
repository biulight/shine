use crate::shells::ShellType;
use anyhow::{Context, Result, bail};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Config {
    /// Presets directory - computed from home
    #[serde(skip)]
    presets_dir: PathBuf,
    /// Bin directory for symlinks - computed from home
    #[serde(skip)]
    bin_dir: PathBuf,
    /// Path to config.toml - computed from home
    #[serde(skip)]
    config_path: PathBuf,
    #[serde(skip)]
    pub home_dir: PathBuf,
    /// Filename
    #[serde(skip)]
    #[allow(dead_code)]
    file_name: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(skip)]
    pub shell_type: ShellType,
}

impl Config {
    pub(crate) async fn load_or_init() -> Result<Self> {
        let home_dir =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir()?;

        let (shine_dir, presets_dir) =
            resolve_runtime_config_dirs(&default_shine_dir, &default_presets_dir).await?;

        let bin_dir = shine_dir.join("bin");
        let config_path = shine_dir.join("config.toml");

        fs::create_dir_all(&shine_dir)
            .await
            .with_context(|| "creating shine config dir")?;
        fs::create_dir_all(&presets_dir)
            .await
            .with_context(|| "creating presets dir")?;
        fs::create_dir_all(&bin_dir)
            .await
            .with_context(|| "creating bin dir")?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;

            let mut config: Config =
                toml::from_str(&contents).context("Failed to parse config file")?;
            config.config_path = config_path.clone();
            config.presets_dir = presets_dir;
            config.bin_dir = bin_dir;
            config.home_dir = home_dir;
            Ok(config)
        } else {
            let config = Config {
                config_path: config_path.clone(),
                presets_dir,
                bin_dir,
                home_dir,
                ..Config::default()
            };
            config.save().await?;
            Ok(config)
        }
    }

    pub(crate) fn presets_dir(&self) -> &Path {
        &self.presets_dir
    }

    pub(crate) fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(dir: &Path) -> Self {
        Self {
            config_path: dir.join("config.toml"),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.to_path_buf(),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn validate(&self) -> Result<()> {
        todo!("Validate config")
    }

    pub(crate) async fn save(&self) -> Result<()> {
        let config_to_save = self.clone();
        let config_path = self.resolve_config_path_for_save().await?;

        let shine_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        let new_toml =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let new_table: toml::Table =
                    toml::from_str(&new_toml).context("Failed to round-trip serialize config")?;
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("Fail to parse existing config for comment preservation")?;

                utils::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        let file_name = config_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let temp_path = shine_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
        let _backup_path = shine_dir.join(format!("{file_name}.bak"));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| format!("Failed to create temp file: {temp_path:?}"))?;

        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to sync temporary config contents")?;
        drop(temp_file);

        fs::rename(&temp_path, &config_path)
            .await
            .with_context(|| format!("Failed to rename temp file to {:?}", config_path))?;

        Ok(())
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

impl Default for Config {
    fn default() -> Self {
        let home_dir =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let shine_dir = home_dir.join(".shine");

        Self {
            presets_dir: shine_dir.join("presets"),
            bin_dir: shine_dir.join("bin"),
            config_path: shine_dir.join("config.toml"),
            home_dir,
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
        }
    }
}

fn default_config_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home).join(".shine"));
    }

    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find user home directory")?;
    Ok(home.join(".shine"))
}

fn default_config_and_presets_dir() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("presets")))
}

async fn resolve_runtime_config_dirs(
    default_shine_dir: &Path,
    default_presets_dir: &Path,
) -> Result<(PathBuf, PathBuf)> {
    if let Ok(custom_config_dir) = std::env::var("SHINE_CONFIG_DIR") {
        let custom_config_dir = custom_config_dir.trim();
        if !custom_config_dir.is_empty() {
            let config_dir = PathBuf::from(shellexpand::tilde(custom_config_dir).to_string());

            return Ok((config_dir.clone(), config_dir.join("presets")));
        }
    }
    Ok((default_shine_dir.to_owned(), default_presets_dir.to_owned()))

    // if let Ok(custom_workspace) = std::env::var("SHINE_WORKSPACE") && !custom_workspace.is_empty() {
    //     let expanded = PathBuf::from(shellexpand::tilde(&custom_workspace).to_string());
    //     return Ok((
    //
    //         ))
    //
    // }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_in(dir: &Path) -> Config {
        Config {
            config_path: dir.join("config.toml"),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.join("home"),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
        }
    }

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn save_writes_config_file_for_new_config() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_integer(), Some(0));

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
    async fn save_merges_preserves_comments() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
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
            presets_dir: PathBuf::from("presets"),
            bin_dir: PathBuf::from("bin"),
            home_dir: PathBuf::from("home"),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
        };
        assert!(config.save().await.is_err());
    }

    #[test]
    fn test_resolve_runtime_config_dir() {}

    #[test]
    fn new_for_test_bin_dir_is_under_root() {
        let dir = std::env::temp_dir().join("shine-test-bin-dir");
        let config = Config::new_for_test(&dir);
        assert_eq!(config.bin_dir(), dir.join("bin"));
    }

    #[tokio::test]
    async fn load_or_init_creates_bin_dir() {
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();
        assert!(config.bin_dir().exists(), "bin dir should be created");
        assert_eq!(config.bin_dir(), dir.join("bin"));

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
