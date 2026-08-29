use super::{SysDriverKind, SysItemStatus};
use crate::permission::PermissionDeclarationV1;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SysManifest {
    pub version: Option<u32>,
    #[serde(default)]
    pub description: String,
    pub default_profile: Option<String>,
    #[serde(default)]
    pub items: Vec<SysItem>,
    #[serde(default)]
    pub profiles: BTreeMap<String, SysProfile>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SysItem {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub mode: SysItemMode,
    #[serde(default)]
    pub requires_admin: bool,
    #[serde(default)]
    pub required_env: Vec<String>,
    #[serde(default)]
    pub driver: SysDriverKind,
    #[serde(default)]
    pub config: toml::Table,
    #[serde(default)]
    pub detect: Option<SysDetection>,
    #[serde(default)]
    pub install: Option<SysInstall>,
    #[serde(default)]
    pub shell: Vec<SysShellIntegration>,
    #[serde(default)]
    pub permissions: Option<PermissionDeclarationV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SysDetection {
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
pub enum SysDetectionProbe {
    Command { command: String },
    Path { path: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SysInstall {
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
    pub fn success_status(&self) -> SysItemStatus {
        match self {
            Self::Package { success_status, .. } | Self::Script { success_status, .. } => {
                success_status.unwrap_or(SysItemStatus::Installed)
            }
        }
    }
    pub fn success_hint(&self) -> &str {
        match self {
            Self::Package { success_hint, .. } | Self::Script { success_hint, .. } => success_hint,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum SysPackageProvider {
    Homebrew,
    HomebrewCask,
    Apt,
    Winget,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SysShellIntegration {
    pub shells: Vec<SysShellKind>,
    pub phase: SysProfilePhase,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub when_command: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, rename = "eval")]
    pub eval_argv: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub fragment: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SysShellKind {
    Bash,
    Zsh,
    Powershell,
}

impl SysShellKind {
    pub fn from_runtime(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "powershell" => Some(Self::Powershell),
            _ => None,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "powershell",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SysItemMode {
    #[default]
    Init,
    Managed,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SysProfile {
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct LoadedSysPreset {
    pub manifest: SysManifest,
    pub root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionSource {
    Items,
    Profile(String),
    DefaultProfile(String),
    Interactive,
    NoItems,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSelection {
    pub item_ids: Vec<String>,
    pub source: SelectionSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysItemOutcome {
    pub item_id: String,
    pub label: String,
    pub status: SysItemStatus,
    pub detail: String,
    pub logs: Vec<String>,
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
pub struct SysInstalledRow {
    pub item_id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum SysProfilePhase {
    Pre,
    Post,
}

pub const SYS_PROFILE_PHASES: [SysProfilePhase; 2] = [SysProfilePhase::Pre, SysProfilePhase::Post];
impl SysProfilePhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellProfileBlockPosition {
    Start,
    End,
}

impl SelectionSource {
    pub fn describe(&self) -> String {
        match self {
            Self::Items => "explicit items".to_string(),
            Self::Profile(name) => format!("profile `{name}`"),
            Self::DefaultProfile(name) => format!("default profile `{name}`"),
            Self::Interactive => "interactive selection".to_string(),
            Self::NoItems => "no selectable items".to_string(),
        }
    }
}
