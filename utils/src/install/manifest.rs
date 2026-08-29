use crate::runtime::FileSystemHost;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MANIFEST_FILE: &str = "app-manifest.toml";
pub const APP_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<AppEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for AppManifest {
    fn default() -> Self {
        Self {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum AppInstallStrategy {
    #[default]
    Copy,
    JsonMerge {
        managed_keys: Vec<String>,
    },
}

impl AppInstallStrategy {
    pub fn is_copy(&self) -> bool {
        matches!(self, Self::Copy)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub source: String,
    pub destination: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    pub content_hash: u64,
    #[serde(default, skip_serializing_if = "AppInstallStrategy::is_copy")]
    pub install_strategy: AppInstallStrategy,
    /// True when the `template` transform was applied during install.
    /// Used by config upgrade to skip files that never used env vars.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uses_env: bool,
    /// True when installing/removing this file requires elevated (sudo) permissions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_admin: bool,
}

pub fn hash_content(bytes: &[u8]) -> u64 {
    // FNV-1a: stable across Rust versions, unlike DefaultHasher
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    bytes.iter().fold(FNV_OFFSET, |hash, &byte| {
        (hash ^ (byte as u64)).wrapping_mul(FNV_PRIME)
    })
}

impl AppManifest {
    pub async fn load(host: &impl FileSystemHost, shine_dir: &Path) -> Result<Self> {
        let path = shine_dir.join(MANIFEST_FILE);
        let mut manifest: Self = match host.read(&path).await {
            Ok(bytes) => toml::from_slice(&bytes)?,
            Err(error) if error.is_not_found() => Self::default(),
            Err(error) => return Err(error.into_anyhow("failed to read app manifest")),
        };
        match manifest.schema_version {
            0 => manifest.schema_version = APP_MANIFEST_SCHEMA_VERSION,
            APP_MANIFEST_SCHEMA_VERSION => {}
            version => bail!(
                "app manifest schema version {version} is newer than this Shine supports ({APP_MANIFEST_SCHEMA_VERSION})"
            ),
        }
        Ok(manifest)
    }

    pub async fn save(&self, host: &impl FileSystemHost, shine_dir: &Path) -> Result<()> {
        if self.schema_version != APP_MANIFEST_SCHEMA_VERSION {
            bail!(
                "cannot write app manifest schema version {}; expected {APP_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        let bytes = toml::to_string_pretty(self)?;
        host.write_atomic(&shine_dir.join(MANIFEST_FILE), bytes.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("failed to write app manifest"))
    }

    pub fn upsert(&mut self, entry: AppEntry) {
        self.entries.retain(|existing| {
            existing.destination != entry.destination && existing.source != entry.source
        });
        self.entries.push(entry);
    }

    pub fn remove_by_dest(&mut self, dest: &Path) -> Option<AppEntry> {
        if let Some(pos) = self.entries.iter().position(|e| e.destination == dest) {
            Some(self.entries.remove(pos))
        } else {
            None
        }
    }

    pub fn find_by_dest(&self, dest: &Path) -> Option<&AppEntry> {
        self.entries.iter().find(|e| e.destination == dest)
    }

    pub fn find_by_source(&self, source: &str) -> Option<&AppEntry> {
        self.entries.iter().find(|entry| entry.source == source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("shine-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).await.unwrap();
        path
    }

    fn sample_entry(dest: &str) -> AppEntry {
        AppEntry {
            source: format!(
                "app/test/{}",
                Path::new(dest).file_name().unwrap().to_string_lossy()
            ),
            destination: PathBuf::from(dest),
            backup: None,
            content_hash: 42,
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    }

    #[tokio::test]
    async fn load_returns_empty_when_missing() {
        let dir = make_temp_dir().await;
        let manifest = AppManifest::load(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();
        assert!(manifest.entries.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = make_temp_dir().await;
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/foo.toml"));
        manifest
            .save(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();

        let loaded = AppManifest::load(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();
        assert_eq!(loaded.schema_version, APP_MANIFEST_SCHEMA_VERSION);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.entries[0].destination,
            PathBuf::from("/tmp/foo.toml")
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn legacy_unversioned_manifest_normalizes_and_writes_version_one() {
        let dir = make_temp_dir().await;
        fs::write(
            dir.join(MANIFEST_FILE),
            r#"[[entries]]
source = "app/test/foo.toml"
destination = "/tmp/foo.toml"
content_hash = 7
"#,
        )
        .await
        .unwrap();

        let manifest = AppManifest::load(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();
        assert_eq!(manifest.schema_version, APP_MANIFEST_SCHEMA_VERSION);
        let after_read = fs::read_to_string(dir.join(MANIFEST_FILE)).await.unwrap();
        assert!(!after_read.contains("schema_version"));
        manifest
            .save(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();

        let written = fs::read_to_string(dir.join(MANIFEST_FILE)).await.unwrap();
        assert!(written.contains("schema_version = 1"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn future_manifest_version_fails_before_use() {
        let dir = make_temp_dir().await;
        fs::write(dir.join(MANIFEST_FILE), "schema_version = 2\n")
            .await
            .unwrap();

        let error = AppManifest::load(&crate::runtime::RealHost, &dir)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("newer than this Shine supports"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_adds_new_entry() {
        let dir = make_temp_dir().await;
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/a.toml"));
        manifest.upsert(sample_entry("/tmp/b.toml"));
        manifest
            .save(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();

        let loaded = AppManifest::load(&crate::runtime::RealHost, &dir)
            .await
            .unwrap();
        assert_eq!(loaded.entries.len(), 2);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn upsert_updates_existing_entry_by_destination() {
        let mut manifest = AppManifest::default();
        manifest.upsert(AppEntry {
            source: "app/x/foo.toml".to_string(),
            destination: PathBuf::from("/tmp/foo.toml"),
            backup: None,
            content_hash: 1,
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        });
        manifest.upsert(AppEntry {
            source: "app/x/foo.toml".to_string(),
            destination: PathBuf::from("/tmp/foo.toml"),
            backup: None,
            content_hash: 2,
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        });
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].content_hash, 2);
    }

    #[test]
    fn upsert_relocates_existing_entry_by_source() {
        let mut manifest = AppManifest::default();
        let old = sample_entry("/tmp/old.toml");
        let mut relocated = sample_entry("/tmp/new.toml");
        relocated.source = old.source.clone();
        manifest.upsert(old.clone());
        manifest.upsert(relocated);
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].destination, Path::new("/tmp/new.toml"));
        assert!(manifest.find_by_source(&old.source).is_some());
    }

    #[test]
    fn remove_by_dest_removes_matching_entry() {
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/a.toml"));
        manifest.upsert(sample_entry("/tmp/b.toml"));
        let removed = manifest.remove_by_dest(Path::new("/tmp/a.toml"));
        assert!(removed.is_some());
        assert_eq!(manifest.entries.len(), 1);
    }

    #[test]
    fn remove_by_dest_is_no_op_for_missing_entry() {
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/a.toml"));
        let removed = manifest.remove_by_dest(Path::new("/tmp/nonexistent.toml"));
        assert!(removed.is_none());
        assert_eq!(manifest.entries.len(), 1);
    }

    #[test]
    fn find_by_dest_returns_entry() {
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/a.toml"));
        assert!(manifest.find_by_dest(Path::new("/tmp/a.toml")).is_some());
        assert!(
            manifest
                .find_by_dest(Path::new("/tmp/other.toml"))
                .is_none()
        );
    }

    #[test]
    fn hash_content_is_deterministic() {
        let h1 = hash_content(b"hello");
        let h2 = hash_content(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_content_differs_for_different_inputs() {
        let h1 = hash_content(b"hello");
        let h2 = hash_content(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn install_strategy_defaults_to_copy() {
        let entry: AppEntry = toml::from_str(
            r#"
source = "app/test/foo.toml"
destination = "/tmp/foo.toml"
content_hash = 7
"#,
        )
        .unwrap();

        assert_eq!(entry.install_strategy, AppInstallStrategy::Copy);
    }
}
