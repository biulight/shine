//! Persisted record of sys item outcomes (`~/.shine/sys-manifest.toml`), keyed
//! by `(os_id, item_id)`. Read by `sys::handle_list`/`handle_info`/`handle_status`
//! to show recorded status; written by the init/apply/uninstall execution paths.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::model::SysItemStatus;
use super::resources::SystemReceipt;

pub(super) const SYS_MANIFEST_FILE: &str = "sys-manifest.toml";
pub(super) const SYS_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SysRunManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) entries: Vec<SysRunEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for SysRunManifest {
    fn default() -> Self {
        Self {
            schema_version: SYS_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
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
    /// Whether this init item contributes its item-owned integrations to the
    /// Rust-composed sys shell profile. Defaults true for legacy entries so a
    /// preset migration does not silently deactivate previously bootstrapped tools.
    #[serde(default = "default_profile_enabled")]
    pub(super) profile_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) receipt: Option<SystemReceipt>,
}

fn default_profile_enabled() -> bool {
    true
}

impl SysRunManifest {
    pub(super) async fn load(shine_dir: &Path) -> Result<Self> {
        let mut manifest: Self = crate::persist::load_toml_or_default(
            &shine_dir.join(SYS_MANIFEST_FILE),
            "sys manifest",
        )
        .await?;
        match manifest.schema_version {
            0 => manifest.schema_version = SYS_MANIFEST_SCHEMA_VERSION,
            SYS_MANIFEST_SCHEMA_VERSION => {}
            version => bail!(
                "sys manifest schema version {version} is newer than this Shine supports ({SYS_MANIFEST_SCHEMA_VERSION})"
            ),
        }
        Ok(manifest)
    }

    pub(super) async fn save(&self, shine_dir: &Path) -> Result<()> {
        if self.schema_version != SYS_MANIFEST_SCHEMA_VERSION {
            bail!(
                "cannot write sys manifest schema version {}; expected {SYS_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        crate::persist::save_toml_atomic(self, &shine_dir.join(SYS_MANIFEST_FILE), "sys manifest")
            .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sys_manifest_legacy_and_future_versions_are_gated() {
        let root = crate::test_support::make_temp_dir("shine-sys-manifest-version").await;
        let path = root.join(SYS_MANIFEST_FILE);
        tokio::fs::write(&path, "entries = []\n").await.unwrap();

        let legacy = SysRunManifest::load(&root).await.unwrap();
        assert_eq!(legacy.schema_version, SYS_MANIFEST_SCHEMA_VERSION);
        let after_read = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!after_read.contains("schema_version"));
        legacy.save(&root).await.unwrap();
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(written.contains("schema_version = 1"));

        tokio::fs::write(&path, "schema_version = 2\nentries = []\n")
            .await
            .unwrap();
        let error = SysRunManifest::load(&root).await.unwrap_err();
        assert!(error.to_string().contains("newer than this Shine supports"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
