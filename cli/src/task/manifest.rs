//! Persisted registry of personal shortcut tasks (`<shine_dir>/tasks.toml`),
//! keyed by task name. Read/written by the `shine task` handlers. Machine-managed
//! runtime state (not an embedded preset), so it follows `SHINE_CONFIG_DIR` via
//! `Config::shine_dir()` and does not preserve user-authored comments.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Optional fixed working directory. Missing for legacy and dynamic-cwd
    /// tasks, which continue to run in the caller's current directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
}

impl TaskManifest {
    pub async fn load(shine_dir: &Path) -> Result<Self> {
        crate::persist::load_toml_or_default(&shine_dir.join(TASKS_FILE), "tasks.toml").await
    }

    pub async fn save(&self, shine_dir: &Path) -> Result<()> {
        crate::persist::save_toml_atomic(self, &shine_dir.join(TASKS_FILE), "tasks.toml").await
    }

    pub fn get(&self, name: &str) -> Option<&TaskEntry> {
        self.tasks.get(name)
    }

    pub fn upsert(&mut self, name: impl Into<String>, command: Vec<String>, cwd: Option<PathBuf>) {
        self.tasks.insert(name.into(), TaskEntry { command, cwd });
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
            None,
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

    #[test]
    fn legacy_entry_without_cwd_deserializes_as_dynamic() {
        let manifest: TaskManifest =
            toml::from_str("[tasks.build]\ncommand = [\"cargo\", \"build\"]\n").unwrap();
        assert_eq!(manifest.get("build").unwrap().cwd, None);
    }

    #[tokio::test]
    async fn save_then_load_roundtrips_fixed_cwd() {
        let dir = temp_dir().await;
        let cwd = dir.join("project");
        let mut manifest = TaskManifest::default();
        manifest.upsert(
            "build",
            vec!["cargo".to_string(), "build".to_string()],
            Some(cwd.clone()),
        );
        manifest.save(&dir).await.unwrap();

        let reloaded = TaskManifest::load(&dir).await.unwrap();
        assert_eq!(reloaded.get("build").unwrap().cwd.as_ref(), Some(&cwd));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_replaces_and_remove_reports_presence() {
        let dir = temp_dir().await;
        let mut manifest = TaskManifest::default();
        manifest.upsert("t", vec!["echo".to_string(), "one".to_string()], None);
        manifest.upsert("t", vec!["echo".to_string(), "two".to_string()], None);
        assert_eq!(manifest.get("t").unwrap().command, ["echo", "two"]);
        assert!(manifest.remove("t"));
        assert!(!manifest.remove("t"));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
