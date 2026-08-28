use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const SHELL_MANIFEST_FILE: &str = "shell-manifest.toml";
pub const SHELL_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalShellMode {
    #[default]
    Snapshot,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkRuntime {
    #[default]
    Native,
    Bun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

impl FromStr for ShellType {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let shell_name = value
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase();
        match shell_name.trim_end_matches(".exe") {
            "bash" => Ok(Self::Bash),
            "fish" => Ok(Self::Fish),
            "zsh" => Ok(Self::Zsh),
            "powershell" | "pwsh" => Ok(Self::PowerShell),
            "elvish" => Ok(Self::Elvish),
            _ => bail!("Unknown shell item type: {value}"),
        }
    }
}

impl From<ShellType> for &'static str {
    fn from(value: ShellType) -> Self {
        match value {
            ShellType::Bash => "bash",
            ShellType::Fish => "fish",
            ShellType::Zsh => "zsh",
            ShellType::PowerShell => "powershell",
            ShellType::Elvish => "elvish",
        }
    }
}

impl Default for ShellType {
    fn default() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Zsh
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellCategory {
    pub name: String,
    pub description: Option<String>,
    pub files: Vec<ShellFile>,
    pub uses_metadata: bool,
}

#[derive(Debug, Clone)]
pub struct ShellFile {
    pub source_rel: PathBuf,
    pub command_name: String,
    pub description: Vec<String>,
    pub needs_source: bool,
    pub runtime: LinkRuntime,
    pub transforms: Vec<String>,
    pub env: Vec<crate::env::EnvVarSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellTarget<'a> {
    pub category: &'a str,
    pub command: Option<&'a str>,
}

pub fn parse_shell_lifecycle_target(target: &str) -> Result<ShellTarget<'_>> {
    let target = target.trim();
    if target.is_empty() {
        bail!("shell preset target must not be empty");
    }
    let mut parts = target.split('/');
    let category = parts.next().unwrap_or_default();
    let command = parts.next();
    if category.is_empty() || command.is_some_and(str::is_empty) || parts.next().is_some() {
        bail!(
            "invalid shell preset target `{target}`; expected <category> or <category>/<command>"
        );
    }
    Ok(ShellTarget { category, command })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellManifestEntry {
    pub category: String,
    pub command: String,
    pub mode: ExternalShellMode,
    pub source_path: PathBuf,
    pub rendered_path: PathBuf,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bun_dependencies: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_hash: Option<u64>,
    #[serde(default)]
    pub transforms: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub needs_source: bool,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<ShellManifestEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for ShellManifest {
    fn default() -> Self {
        Self {
            schema_version: SHELL_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

impl ShellManifest {
    pub async fn load(shine_dir: &(impl AsRef<Path> + ?Sized)) -> Result<Self> {
        let shine_dir = shine_dir.as_ref();
        let mut manifest: Self = crate::persist::load_toml_or_default(
            &shine_dir.join(SHELL_MANIFEST_FILE),
            "shell manifest",
        )
        .await?;
        match manifest.schema_version {
            0 => manifest.schema_version = SHELL_MANIFEST_SCHEMA_VERSION,
            SHELL_MANIFEST_SCHEMA_VERSION => {}
            version => bail!(
                "shell manifest schema version {version} is newer than this Shine supports ({SHELL_MANIFEST_SCHEMA_VERSION})"
            ),
        }
        Ok(manifest)
    }

    pub async fn save(&self, shine_dir: &(impl AsRef<Path> + ?Sized)) -> Result<()> {
        let shine_dir = shine_dir.as_ref();
        if self.schema_version != SHELL_MANIFEST_SCHEMA_VERSION {
            bail!(
                "cannot write shell manifest schema version {}; expected {SHELL_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        crate::persist::save_toml_atomic(
            self,
            &shine_dir.join(SHELL_MANIFEST_FILE),
            "shell manifest",
        )
        .await
    }

    pub fn find(&self, target: &str) -> Option<&ShellManifestEntry> {
        self.entries
            .iter()
            .find(|entry| canonical_target(entry) == target)
    }

    pub fn replace_categories(
        &mut self,
        categories: &BTreeSet<String>,
        entries: Vec<ShellManifestEntry>,
    ) {
        self.entries
            .retain(|entry| !categories.contains(&entry.category));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }

    pub fn remove_category(&mut self, category: &str) {
        self.entries.retain(|entry| entry.category != category);
    }

    pub fn remove_target(&mut self, category: &str, command: &str) {
        self.entries
            .retain(|entry| entry.category != category || entry.command != command);
    }

    pub fn replace_targets(
        &mut self,
        targets: &BTreeSet<String>,
        entries: Vec<ShellManifestEntry>,
    ) {
        self.entries
            .retain(|entry| !targets.contains(&canonical_target(entry)));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }
}

fn canonical_target(entry: &ShellManifestEntry) -> String {
    format!("shell/{}/{}", entry.category, entry.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_and_future_versions_are_gated_in_core() {
        let root =
            std::env::temp_dir().join(format!("shine-shell-manifest-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join(SHELL_MANIFEST_FILE);
        tokio::fs::write(&path, "entries = []\n").await.unwrap();

        let legacy = ShellManifest::load(&root).await.unwrap();
        assert_eq!(legacy.schema_version, SHELL_MANIFEST_SCHEMA_VERSION);
        assert!(
            !tokio::fs::read_to_string(&path)
                .await
                .unwrap()
                .contains("schema_version")
        );
        legacy.save(&root).await.unwrap();
        assert!(
            tokio::fs::read_to_string(&path)
                .await
                .unwrap()
                .contains("schema_version = 1")
        );

        tokio::fs::write(&path, "schema_version = 2\nentries = []\n")
            .await
            .unwrap();
        assert!(
            ShellManifest::load(&root)
                .await
                .unwrap_err()
                .to_string()
                .contains("newer")
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
