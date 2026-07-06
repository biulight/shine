//! Persisted record of sys item outcomes (`~/.shine/sys-manifest.toml`), keyed
//! by `(os_id, item_id)`. Read by `sys::handle_list`/`handle_info`/`handle_status`
//! to show recorded status; written by the init/apply/uninstall execution paths.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncWriteExt;

use super::model::SysItemStatus;
use super::resources::SystemReceipt;

pub(super) const SYS_MANIFEST_FILE: &str = "sys-manifest.toml";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SysRunManifest {
    #[serde(default)]
    pub(super) entries: Vec<SysRunEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SysRunEntry {
    pub(super) os_id: String,
    pub(super) item_id: String,
    pub(super) label: String,
    pub(super) status: SysItemStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) detail: String,
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) receipt: Option<SystemReceipt>,
}

impl SysRunManifest {
    pub(super) async fn load(shine_dir: &Path) -> Result<Self> {
        let path = shine_dir.join(SYS_MANIFEST_FILE);
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => toml::from_str(&content).context("failed to parse sys manifest"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("failed to read sys manifest"),
        }
    }

    pub(super) async fn save(&self, shine_dir: &Path) -> Result<()> {
        tokio::fs::create_dir_all(shine_dir)
            .await
            .with_context(|| format!("creating {}", shine_dir.display()))?;

        let path = shine_dir.join(SYS_MANIFEST_FILE);
        let content = toml::to_string_pretty(self).context("failed to serialize sys manifest")?;
        let temp = shine_dir.join(format!(".sys-manifest-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .await
            .context("failed to create temp sys manifest file")?;
        file.write_all(content.as_bytes())
            .await
            .context("failed to write sys manifest")?;
        file.sync_all()
            .await
            .context("failed to sync sys manifest")?;
        drop(file);

        tokio::fs::rename(&temp, &path)
            .await
            .context("failed to finalize sys manifest")?;
        Ok(())
    }

    pub(super) fn upsert(&mut self, entry: SysRunEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.os_id == entry.os_id && existing.item_id == entry.item_id)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }
}
