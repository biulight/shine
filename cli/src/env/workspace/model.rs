//! Model for workspace environments.
use super::file_groups;
use super::{
    BTreeMap, Context, Deserialize, ENV_SOURCE_FORMAT_VERSION, Path, Result, SensitiveValues,
    Serialize, WORKSPACE_FORMAT_VERSION, Zeroize, bail, clean_recipients, secret,
};

#[derive(Clone, Debug, Deserialize)]
pub struct Workspace {
    #[serde(default = "workspace_format_version")]
    pub(super) version: u32,
    pub env: WorkspaceEnv,
    #[serde(default)]
    pub(super) file_groups: BTreeMap<String, file_groups::FileGroup>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkspaceEnv {
    #[serde(default)]
    pub(super) default_mode: Option<String>,
    #[serde(default)]
    pub(super) modes: Vec<String>,
    #[serde(default)]
    pub(super) files: Vec<String>,
    #[serde(default)]
    pub(super) override_process_env: bool,
    #[serde(default)]
    pub(super) encryption: Encryption,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct Encryption {
    #[serde(rename = "recipient")]
    pub(super) legacy_recipient: Option<String>,
    #[serde(default)]
    pub(super) gpg_recipients: Vec<String>,
    #[serde(default)]
    pub(super) backend: Option<String>,
    #[serde(default)]
    pub(super) age_recipients: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SourceFile {
    #[serde(default = "env_source_format_version")]
    pub(super) version: u32,
    #[serde(default)]
    pub(super) plain: SensitiveValues,
    #[serde(default)]
    pub(super) secret: BTreeMap<String, SecretState>,
    #[serde(default)]
    pub(super) payload: PayloadField,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum SecretState {
    Sealed(bool),
    Plain(String),
}

impl std::fmt::Debug for SecretState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sealed(sealed) => f.debug_tuple("Sealed").field(sealed).finish(),
            Self::Plain(_) => f.write_str("Plain([redacted])"),
        }
    }
}

impl Drop for SecretState {
    fn drop(&mut self) {
        if let Self::Plain(value) = self {
            value.zeroize();
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct PayloadField {
    #[serde(default)]
    pub(super) data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SecretPayload {
    pub(super) version: u32,
    pub(super) values: SensitiveValues,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct CacheFile {
    pub(super) version: u32,
    pub(super) project_root: String,
    pub(super) modes: BTreeMap<String, CachedMode>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct CachedMode {
    pub(super) input_hash: String,
    pub(super) keys: Vec<String>,
    pub(super) data: String,
}

pub(super) fn workspace_format_version() -> u32 {
    WORKSPACE_FORMAT_VERSION
}

pub(super) fn env_source_format_version() -> u32 {
    ENV_SOURCE_FORMAT_VERSION
}

pub(super) fn parse_workspace(path: &Path, contents: &str) -> Result<Workspace> {
    let workspace: Workspace = toml::from_str(contents)
        .map_err(crate::config::config_parse_error)
        .with_context(|| format!("parsing {}", path.display()))?;
    if workspace.version < WORKSPACE_FORMAT_VERSION {
        bail!(
            "workspace version {} in {} is retired; run `shine state migrate`",
            workspace.version,
            path.display()
        );
    }
    if !matches!(workspace.version, 2 | 3) {
        bail!(
            "unsupported workspace version {} in {}",
            workspace.version,
            path.display()
        );
    }
    if workspace.env.encryption.legacy_recipient.is_some() {
        bail!(
            "{} uses retired env.encryption.recipient; run `shine state migrate` to convert it to gpg_recipients",
            path.display()
        );
    }
    if workspace.version == 2 && !workspace.file_groups.is_empty() {
        bail!("file_groups requires workspace version 3");
    }
    if workspace.env.files.is_empty()
        && (workspace.version == 2 || workspace.file_groups.is_empty())
    {
        bail!(
            "env.files must contain at least one source path unless version 3 defines file_groups"
        );
    }
    for (name, group) in &workspace.file_groups {
        validate_mode(name).context("invalid file group name")?;
        if group.files.is_empty() {
            bail!("file group {name} must contain files");
        }
    }
    if workspace
        .env
        .encryption
        .backend
        .as_deref()
        .is_some_and(|b| b.trim().eq_ignore_ascii_case("hybrid"))
    {
        secret::hybrid::Recipients::new(
            clean_recipients(&workspace.env.encryption.gpg_recipients),
            clean_recipients(&workspace.env.encryption.age_recipients),
        )?;
    }
    Ok(workspace)
}

pub(super) fn validate_mode(mode: &str) -> Result<()> {
    if mode.is_empty()
        || !mode
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("mode must contain only letters, digits, hyphens, and underscores");
    }
    Ok(())
}

pub(crate) fn validate_broker_mode(mode: &str) -> Result<()> {
    validate_mode(mode)
}

pub(super) fn parse_source(path: &Path, contents: &str) -> Result<SourceFile> {
    let source: SourceFile = toml::from_str(contents)
        .map_err(|_| anyhow::anyhow!("invalid environment source syntax in {}", path.display()))?;
    if source.version != ENV_SOURCE_FORMAT_VERSION {
        bail!(
            "unsupported environment source version {} in {}",
            source.version,
            path.display()
        );
    }
    for key in source.plain.keys().chain(source.secret.keys()) {
        crate::env::validate_env_key(key)?;
    }
    if let Some(key) = source
        .plain
        .keys()
        .find(|key| source.secret.contains_key(*key))
    {
        bail!(
            "{key} appears in both [plain] and [secret] in {}",
            path.display()
        );
    }
    Ok(source)
}
