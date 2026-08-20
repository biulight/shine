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
    /// Opts this OS preset into Rust-composed base + item shell integrations.
    /// Absent/false preserves the legacy platform-wide profile templates.
    #[serde(default)]
    pub(super) profile_composition: bool,
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
    #[serde(default)]
    pub(super) detect: Option<SysDetection>,
    #[serde(default)]
    pub(super) install: Option<SysInstall>,
    #[serde(default)]
    pub(super) shell: Vec<SysShellIntegration>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum SysDetection {
    Command {
        command: String,
        #[serde(default)]
        version_args: Vec<String>,
    },
    Path {
        path: String,
    },
    Any {
        probes: Vec<SysDetectionProbe>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum SysDetectionProbe {
    Command { command: String },
    Path { path: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum SysInstall {
    Package {
        provider: SysPackageProvider,
        package: String,
        #[serde(default)]
        success_status: Option<SysItemStatus>,
        #[serde(default)]
        success_hint: String,
    },
    Script {
        path: String,
        #[serde(default)]
        success_status: Option<SysItemStatus>,
        #[serde(default)]
        success_hint: String,
    },
}

impl SysInstall {
    pub(super) fn success_status(&self) -> SysItemStatus {
        match self {
            Self::Package { success_status, .. } | Self::Script { success_status, .. } => {
                success_status.unwrap_or(SysItemStatus::Installed)
            }
        }
    }

    pub(super) fn success_hint(&self) -> &str {
        match self {
            Self::Package { success_hint, .. } | Self::Script { success_hint, .. } => success_hint,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SysPackageProvider {
    Homebrew,
    HomebrewCask,
    Apt,
    Winget,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct SysShellIntegration {
    pub(super) shells: Vec<SysShellKind>,
    pub(super) phase: SysProfilePhase,
    #[serde(default)]
    pub(super) priority: i32,
    #[serde(default)]
    pub(super) when_command: Option<String>,
    #[serde(default)]
    pub(super) path: Option<String>,
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
    #[serde(default, rename = "eval")]
    pub(super) eval_argv: Vec<String>,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) fragment: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SysShellKind {
    Bash,
    Zsh,
    Powershell,
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
    Items,
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

/// Ephemeral result of a bootstrap-software update check. Unlike
/// `SysItemStatus`, this is deliberately never persisted in sys-manifest.toml.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SysUpdateState {
    Available,
    Current,
    Manual,
    Unsupported,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SysUpdateCheck {
    pub(super) item_id: String,
    pub(super) label: String,
    pub(super) state: SysUpdateState,
    pub(super) detail: String,
    pub(super) upgrade_command: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SysInstalledRow {
    pub item_id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
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
            Self::Items => "explicit items".to_string(),
            Self::Profile(name) => format!("profile `{name}`"),
            Self::DefaultProfile(name) => format!("default profile `{name}`"),
            Self::Interactive => "interactive selection".to_string(),
            Self::NoItems => "no selectable items".to_string(),
        }
    }
}
