//! Persisted record of sys item outcomes (`~/.shine/sys-manifest.toml`), keyed
//! by `(os_id, item_id)`. Read by `sys::handle_list`/`handle_info`/`handle_status`
//! to show recorded status; written by the init/apply/uninstall execution paths.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
        crate::persist::load_toml_or_default(&shine_dir.join(SYS_MANIFEST_FILE), "sys manifest")
            .await
    }

    pub(super) async fn save(&self, shine_dir: &Path) -> Result<()> {
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
