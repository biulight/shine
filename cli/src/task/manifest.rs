//! Persisted registry of personal shortcut tasks (`<shine_dir>/tasks.toml`),
//! keyed by task name. Read/written by the `shine task` handlers. Machine-managed
//! runtime state (not an embedded preset), so it follows `SHINE_CONFIG_DIR` via
//! `Config::shine_dir()` and does not preserve user-authored comments.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub(crate) const TASKS_FILE: &str = "tasks.toml";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskManifest {
    /// `BTreeMap` keeps `tasks.toml` and `shine task list` in stable name order.
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskEntry {
    /// Command and arguments stored as an argv array, preserving argument
    /// boundaries. Executed directly (no shell) unless the user saved an
    /// explicit `sh -c ...` invocation.
    pub command: Vec<String>,
}

impl TaskManifest {
    pub async fn load(shine_dir: &Path) -> Result<Self> {
        let path = shine_dir.join(TASKS_FILE);
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => toml::from_str(&content).context("failed to parse tasks.toml"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("failed to read tasks.toml"),
        }
    }

    pub async fn save(&self, shine_dir: &Path) -> Result<()> {
        tokio::fs::create_dir_all(shine_dir)
            .await
            .with_context(|| format!("creating {}", shine_dir.display()))?;

        let path = shine_dir.join(TASKS_FILE);
        let content = toml::to_string_pretty(self).context("failed to serialize tasks.toml")?;
        let temp = shine_dir.join(format!(".tasks-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .await
            .context("failed to create temp tasks file")?;
        file.write_all(content.as_bytes())
            .await
            .context("failed to write tasks.toml")?;
        file.sync_all().await.context("failed to sync tasks.toml")?;
        drop(file);

        tokio::fs::rename(&temp, &path)
            .await
            .context("failed to finalize tasks.toml")?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&TaskEntry> {
        self.tasks.get(name)
    }

    pub fn upsert(&mut self, name: impl Into<String>, command: Vec<String>) {
        self.tasks.insert(name.into(), TaskEntry { command });
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.tasks.remove(name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-task-manifest").await
    }

    #[tokio::test]
    async fn load_returns_default_when_file_missing() {
        let dir = temp_dir().await;
        let manifest = TaskManifest::load(&dir).await.unwrap();
        assert!(manifest.tasks.is_empty());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_then_load_roundtrips_argv_boundaries() {
        let dir = temp_dir().await;
        let mut manifest = TaskManifest::default();
        manifest.upsert(
            "kill-port",
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "lsof -ti :3000 | xargs kill".to_string(),
            ],
        );
        manifest.save(&dir).await.unwrap();

        let reloaded = TaskManifest::load(&dir).await.unwrap();
        assert_eq!(manifest, reloaded);
        assert_eq!(
            reloaded.get("kill-port").unwrap().command,
            ["sh", "-c", "lsof -ti :3000 | xargs kill"]
        );
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_replaces_and_remove_reports_presence() {
        let dir = temp_dir().await;
        let mut manifest = TaskManifest::default();
        manifest.upsert("t", vec!["echo".to_string(), "one".to_string()]);
        manifest.upsert("t", vec!["echo".to_string(), "two".to_string()]);
        assert_eq!(manifest.get("t").unwrap().command, ["echo", "two"]);
        assert!(manifest.remove("t"));
        assert!(!manifest.remove("t"));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
