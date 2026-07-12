//! Data model for sys presets: the on-disk manifest shape (`sys-manifest.toml`
//! under `presets/sys/<os>/`), selection results, and run-outcome types shared
//! across `sys::execution`, `sys::selection`, `sys::profile`, and `sys::manifest`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct SysManifest {
    #[serde(default)]
    pub(super) description: String,
    pub(super) default_profile: Option<String>,
    #[serde(default)]
    pub(super) items: Vec<SysItem>,
    #[serde(default)]
    pub(super) profiles: BTreeMap<String, SysProfile>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SysItem {
    pub(super) id: String,
    pub(super) label: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) default: bool,
    #[serde(default)]
    pub(super) mode: SysItemMode,
    #[serde(default)]
    pub(super) requires_admin: bool,
    #[serde(default)]
    pub(super) required_env: Vec<String>,
    #[serde(default)]
    pub(super) driver: SysDriverKind,
    #[serde(default)]
    pub(super) config: toml::Table,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SysDriverKind {
    #[default]
    Script,
    SplitDns,
    ManagedFile,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SysItemMode {
    #[default]
    Init,
    Managed,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct SysProfile {
    #[serde(default)]
    pub(super) items: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct LoadedSysPreset {
    pub(super) manifest: SysManifest,
    pub(super) script_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SysInitCommand {
    pub(super) program: &'static str,
    pub(super) fixed_args: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SelectionSource {
    Profile(String),
    DefaultProfile(String),
    Interactive,
    NoItems,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResolvedSelection {
    pub(super) item_ids: Vec<String>,
    pub(super) source: SelectionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SysItemStatus {
    Installed,
    AlreadyInstalled,
    Skipped,
    Updated,
    NeedsAction,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SysItemOutcome {
    pub(super) item_id: String,
    pub(super) label: String,
    pub(super) status: SysItemStatus,
    pub(super) detail: String,
    pub(super) logs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SysUpgradeReport {
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysUpdateRow {
    pub item_id: String,
    pub label: String,
    pub details: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SysProfilePhase {
    Pre,
    Post,
}

pub(super) const SYS_PROFILE_PHASES: [SysProfilePhase; 2] =
    [SysProfilePhase::Pre, SysProfilePhase::Post];

impl SysProfilePhase {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShellProfileBlockPosition {
    Start,
    End,
}

impl SelectionSource {
    pub(super) fn describe(&self) -> String {
        match self {
            Self::Profile(name) => format!("profile `{name}`"),
            Self::DefaultProfile(name) => format!("default profile `{name}`"),
            Self::Interactive => "interactive selection".to_string(),
            Self::NoItems => "no selectable items".to_string(),
        }
    }
}
