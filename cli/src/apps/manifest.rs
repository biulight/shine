use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const MANIFEST_FILE: &str = "app-manifest.toml";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct AppManifest {
    #[serde(default)]
    pub entries: Vec<AppEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AppEntry {
    pub source: String,
    pub destination: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    pub content_hash: u64,
    /// True when the `template` transform was applied during install.
    /// Used by config upgrade to skip files that never used env vars.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uses_env: bool,
}

pub(crate) fn hash_content(bytes: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

impl AppManifest {
    pub(crate) async fn load(shine_dir: &Path) -> Result<Self> {
        let path = shine_dir.join(MANIFEST_FILE);
        match fs::read_to_string(&path).await {
            Ok(content) => toml::from_str(&content).context("failed to parse app manifest"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("failed to read app manifest"),
        }
    }

    pub(crate) async fn save(&self, shine_dir: &Path) -> Result<()> {
        let path = shine_dir.join(MANIFEST_FILE);
        let content = toml::to_string_pretty(self).context("failed to serialize app manifest")?;

        let temp = shine_dir.join(format!(".app-manifest-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .await
            .context("failed to create temp manifest file")?;
        file.write_all(content.as_bytes())
            .await
            .context("failed to write manifest")?;
        file.sync_all().await.context("failed to sync manifest")?;
        drop(file);

        fs::rename(&temp, &path)
            .await
            .context("failed to finalize manifest")?;
        Ok(())
    }

    pub(crate) fn upsert(&mut self, entry: AppEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.destination == entry.destination)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub(crate) fn remove_by_dest(&mut self, dest: &Path) -> Option<AppEntry> {
        if let Some(pos) = self.entries.iter().position(|e| e.destination == dest) {
            Some(self.entries.remove(pos))
        } else {
            None
        }
    }

    pub(crate) fn find_by_dest(&self, dest: &Path) -> Option<&AppEntry> {
        self.entries.iter().find(|e| e.destination == dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn sample_entry(dest: &str) -> AppEntry {
        AppEntry {
            source: "app/test/foo.toml".to_string(),
            destination: PathBuf::from(dest),
            backup: None,
            content_hash: 42,
            uses_env: false,
        }
    }

    #[tokio::test]
    async fn load_returns_empty_when_missing() {
        let dir = make_temp_dir().await;
        let manifest = AppManifest::load(&dir).await.unwrap();
        assert!(manifest.entries.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = make_temp_dir().await;
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/foo.toml"));
        manifest.save(&dir).await.unwrap();

        let loaded = AppManifest::load(&dir).await.unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.entries[0].destination,
            PathBuf::from("/tmp/foo.toml")
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_adds_new_entry() {
        let dir = make_temp_dir().await;
        let mut manifest = AppManifest::default();
        manifest.upsert(sample_entry("/tmp/a.toml"));
        manifest.upsert(sample_entry("/tmp/b.toml"));
        manifest.save(&dir).await.unwrap();

        let loaded = AppManifest::load(&dir).await.unwrap();
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
            uses_env: false,
        });
        manifest.upsert(AppEntry {
            source: "app/x/foo.toml".to_string(),
            destination: PathBuf::from("/tmp/foo.toml"),
            backup: None,
            content_hash: 2,
            uses_env: false,
        });
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].content_hash, 2);
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
}
