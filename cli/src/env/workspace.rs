use crate::{config::Config, secret};
use anyhow::{Context, Result, bail};
use dialoguer::Password;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};
use tokio::process::Command;
use toml_edit::{DocumentMut, value};

const WORKSPACE_FILE: &str = "shine.workspace.toml";
const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct Workspace {
    #[serde(default = "format_version")]
    version: u32,
    pub env: WorkspaceEnv,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkspaceEnv {
    #[serde(default)]
    default_mode: Option<String>,
    #[serde(default)]
    modes: Vec<String>,
    files: Vec<String>,
    #[serde(default)]
    override_process_env: bool,
    #[serde(default)]
    encryption: Encryption,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Encryption {
    recipient: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SourceFile {
    #[serde(default = "format_version")]
    version: u32,
    #[serde(default)]
    plain: BTreeMap<String, String>,
    #[serde(default)]
    secret: BTreeMap<String, SecretState>,
    #[serde(default)]
    payload: PayloadField,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum SecretState {
    Sealed(bool),
    Plain(String),
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PayloadField {
    #[serde(default)]
    data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SecretPayload {
    version: u32,
    values: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    project_root: String,
    modes: BTreeMap<String, CachedMode>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedMode {
    input_hash: String,
    keys: Vec<String>,
    data: String,
}

fn format_version() -> u32 {
    FORMAT_VERSION
}

pub async fn handle_seal(
    config: &Config,
    workspace_arg: Option<&Path>,
    file: Option<&Path>,
    recipient_arg: Option<&str>,
) -> Result<()> {
    let workspace_path = find_workspace_optional(workspace_arg).await?;
    let workspace = match &workspace_path {
        Some(path) => Some(load_workspace(path).await?),
        None => None,
    };
    let recipient = resolve_seal_recipient(
        recipient_arg,
        workspace
            .as_ref()
            .and_then(|workspace| workspace.env.encryption.recipient.as_deref()),
        config.gpg_key_id.as_deref(),
    );

    let files = if let Some(file) = file {
        vec![absolute_from_current(file)?]
    } else {
        let workspace_path = workspace_path
            .as_deref()
            .context("shine.workspace.toml was not found; pass FILE or --workspace")?;
        let workspace = workspace.as_ref().expect("workspace path has workspace");
        existing_workspace_sources(workspace_path, workspace).await?
    };
    if files.is_empty() {
        bail!("no workspace environment source files were found");
    }

    for path in &files {
        seal_file(path, recipient).await?;
        println!("sealed {}", path.display());
    }
    Ok(())
}

pub async fn handle_run(
    config: &Config,
    workspace_arg: Option<&Path>,
    mode_arg: Option<&str>,
    command: &[OsString],
) -> Result<()> {
    let workspace_path = find_workspace(workspace_arg).await?;
    let workspace = load_workspace(&workspace_path).await?;
    let mode = mode_arg
        .or(workspace.env.default_mode.as_deref())
        .context("environment mode is required; pass --mode or set env.default_mode")?;
    validate_mode(mode)?;
    let sources = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
    let input_hash = calculate_input_hash(&workspace_path, mode, &sources).await?;

    let recipient = resolve_recipient_optional(
        workspace.env.encryption.recipient.as_deref(),
        config.gpg_key_id.as_deref(),
    );
    let cache_path = cache_path(&workspace_path, mode)?;
    let values = match read_valid_cache(&cache_path, mode, &input_hash).await {
        Ok(Some(values)) => values,
        Ok(None) => {
            let values = compile_sources(&sources).await?;
            if let Some(recipient) = recipient
                && let Err(error) = write_cache(
                    &cache_path,
                    &workspace_path,
                    mode,
                    &input_hash,
                    &values,
                    recipient,
                )
                .await
            {
                eprintln!("Warning: could not update environment cache: {error:#}");
            }
            values
        }
        Err(error) => {
            eprintln!("Warning: ignoring unreadable environment cache: {error:#}");
            compile_sources(&sources).await?
        }
    };

    run_command(command, &values, workspace.env.override_process_env).await
}

async fn find_workspace_optional(explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(absolute_from_current(path)?));
    }
    let current = std::env::current_dir().context("reading current directory")?;
    Ok(current
        .ancestors()
        .map(|directory| directory.join(WORKSPACE_FILE))
        .find(|path| path.is_file()))
}

async fn find_workspace(explicit: Option<&Path>) -> Result<PathBuf> {
    find_workspace_optional(explicit)
        .await?
        .context("shine.workspace.toml was not found; pass --workspace")
}

async fn load_workspace(path: &Path) -> Result<Workspace> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let workspace: Workspace =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    if workspace.version != FORMAT_VERSION {
        bail!(
            "unsupported workspace version {} in {}",
            workspace.version,
            path.display()
        );
    }
    if workspace.env.files.is_empty() {
        bail!("env.files must contain at least one source path");
    }
    Ok(workspace)
}

fn resolve_seal_recipient<'a>(
    cli: Option<&'a str>,
    workspace: Option<&'a str>,
    global: Option<&'a str>,
) -> Option<&'a str> {
    resolve_recipient_optional(cli, workspace).or_else(|| resolve_recipient_optional(global, None))
}

fn resolve_recipient_optional<'a>(
    first: Option<&'a str>,
    second: Option<&'a str>,
) -> Option<&'a str> {
    first
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| second.map(str::trim).filter(|value| !value.is_empty()))
}

async fn existing_workspace_sources(path: &Path, workspace: &Workspace) -> Result<Vec<PathBuf>> {
    let mut modes = workspace.env.modes.clone();
    if let Some(default_mode) = &workspace.env.default_mode
        && !modes.contains(default_mode)
    {
        modes.push(default_mode.clone());
    }
    if modes.is_empty()
        && workspace
            .env
            .files
            .iter()
            .any(|file| file.contains("{mode}"))
    {
        bail!("env.modes or env.default_mode is required to seal mode-specific files");
    }
    if modes.is_empty() {
        modes.push(String::new());
    }

    let mut unique = BTreeSet::new();
    for mode in modes {
        for source in resolve_sources(path, &workspace.env.files, &mode)? {
            if source.is_file() {
                unique.insert(source);
            }
        }
    }
    Ok(unique.into_iter().collect())
}

fn resolve_sources(workspace_path: &Path, files: &[String], mode: &str) -> Result<Vec<PathBuf>> {
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    files
        .iter()
        .map(|file| {
            if file.contains("{mode}") && mode.is_empty() {
                bail!("cannot expand {file} without a mode");
            }
            let expanded = file.replace("{mode}", mode);
            let path = PathBuf::from(expanded);
            Ok(if path.is_absolute() {
                path
            } else {
                root.join(path)
            })
        })
        .collect()
}

fn validate_mode(mode: &str) -> Result<()> {
    if mode.is_empty()
        || !mode
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("mode must contain only letters, digits, hyphens, and underscores");
    }
    Ok(())
}

async fn seal_file(path: &Path, recipient: Option<&str>) -> Result<()> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let source = parse_source(path, &contents)?;
    let mut old_values = decrypt_source_payload(path, &source).await?;
    let mut new_values = BTreeMap::new();

    for (key, state) in &source.secret {
        validate_env_key(key)?;
        let secret = match state {
            SecretState::Sealed(true) => old_values
                .remove(key)
                .with_context(|| format!("{key} is marked sealed but is missing from payload"))?,
            SecretState::Sealed(false) => Password::new()
                .with_prompt(format!("Enter {key}"))
                .with_confirmation("Confirm value", "Values did not match")
                .interact()
                .with_context(|| format!("reading {key}"))?,
            SecretState::Plain(value) => value.clone(),
        };
        new_values.insert(key.clone(), secret);
    }

    let encoded = if new_values.is_empty() {
        String::new()
    } else {
        let recipient = recipient.context(
            "GPG recipient is required; pass --recipient, set env.encryption.recipient in shine.workspace.toml, or set gpg_key_id",
        )?;
        let plaintext = toml::to_string(&SecretPayload {
            version: FORMAT_VERSION,
            values: new_values,
        })?;
        secret::encrypt_gpg_secret_to_base64(plaintext.as_bytes(), recipient).await?
    };

    let mut document = contents
        .parse::<DocumentMut>()
        .with_context(|| format!("parsing {} for update", path.display()))?;
    for key in source.secret.keys() {
        let item = &mut document["secret"][key];
        let decor = item.as_value().map(|value| value.decor().clone());
        *item = value(true);
        if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
            *value.decor_mut() = decor;
        }
    }
    if !document.contains_key("payload") {
        document["payload"] = toml_edit::table();
    }
    document["payload"]["data"] = value(encoded);
    atomic_write(path, document.to_string().as_bytes()).await
}

fn parse_source(path: &Path, contents: &str) -> Result<SourceFile> {
    let source: SourceFile = toml::from_str(contents)
        .with_context(|| format!("parsing environment source {}", path.display()))?;
    if source.version != FORMAT_VERSION {
        bail!(
            "unsupported environment source version {} in {}",
            source.version,
            path.display()
        );
    }
    for key in source.plain.keys().chain(source.secret.keys()) {
        validate_env_key(key)?;
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

async fn decrypt_source_payload(
    path: &Path,
    source: &SourceFile,
) -> Result<BTreeMap<String, String>> {
    if source.payload.data.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let plaintext = secret::decrypt_base64_gpg_secret(&source.payload.data)
        .await
        .with_context(|| format!("decrypting {}", path.display()))?;
    let payload: SecretPayload = toml::from_str(&plaintext)
        .with_context(|| format!("parsing decrypted payload from {}", path.display()))?;
    if payload.version != FORMAT_VERSION {
        bail!("unsupported encrypted payload version {}", payload.version);
    }
    Ok(payload.values)
}

async fn load_sealed_source(path: &Path) -> Result<BTreeMap<String, String>> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let source = parse_source(path, &contents)?;
    for (key, state) in &source.secret {
        if !matches!(state, SecretState::Sealed(true)) {
            bail!(
                "{key} in {} is not sealed; run `shine env seal`",
                path.display()
            );
        }
    }
    let secrets = decrypt_source_payload(path, &source).await?;
    let expected: BTreeSet<_> = source.secret.keys().cloned().collect();
    let actual: BTreeSet<_> = secrets.keys().cloned().collect();
    if expected != actual {
        bail!(
            "secret key list does not match encrypted payload in {}",
            path.display()
        );
    }
    let mut values = source.plain;
    values.extend(secrets);
    Ok(values)
}

async fn compile_sources(sources: &[PathBuf]) -> Result<BTreeMap<String, String>> {
    let mut merged = BTreeMap::new();
    let mut loaded = 0usize;
    for path in sources {
        if !path.is_file() {
            continue;
        }
        merged.extend(load_sealed_source(path).await?);
        loaded += 1;
    }
    if loaded == 0 {
        bail!("none of the configured environment source files exist");
    }
    Ok(merged)
}

async fn calculate_input_hash(
    workspace_path: &Path,
    mode: &str,
    sources: &[PathBuf],
) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(FORMAT_VERSION.to_le_bytes());
    hash.update(mode.as_bytes());
    hash.update(
        tokio::fs::read(workspace_path)
            .await
            .with_context(|| format!("reading {}", workspace_path.display()))?,
    );
    for path in sources {
        hash.update(path.to_string_lossy().as_bytes());
        match tokio::fs::read(path).await {
            Ok(contents) => hash.update(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => hash.update(b"<missing>"),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }
    hash.update(workspace_path.to_string_lossy().as_bytes());
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn cache_path(workspace_path: &Path, mode: &str) -> Result<PathBuf> {
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let project_id = format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    );
    let base = BaseDirs::new().context("resolving system cache directory")?;
    Ok(base
        .cache_dir()
        .join("shine")
        .join("projects")
        .join(project_id)
        .join(format!("env-{mode}.toml")))
}

async fn read_valid_cache(
    path: &Path,
    mode: &str,
    input_hash: &str,
) -> Result<Option<BTreeMap<String, String>>> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let cache: CacheFile =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    let Some(cached) = cache.modes.get(mode) else {
        return Ok(None);
    };
    if cache.version != FORMAT_VERSION || cached.input_hash != input_hash {
        return Ok(None);
    }
    let plaintext = secret::decrypt_base64_gpg_secret(&cached.data).await?;
    let payload: SecretPayload = toml::from_str(&plaintext)?;
    let keys: Vec<_> = payload.values.keys().cloned().collect();
    if payload.version != FORMAT_VERSION || keys != cached.keys {
        bail!("compiled environment cache failed integrity validation");
    }
    Ok(Some(payload.values))
}

async fn write_cache(
    path: &Path,
    workspace_path: &Path,
    mode: &str,
    input_hash: &str,
    values: &BTreeMap<String, String>,
    recipient: &str,
) -> Result<()> {
    let plaintext = toml::to_string(&SecretPayload {
        version: FORMAT_VERSION,
        values: values.clone(),
    })?;
    let data = secret::encrypt_gpg_secret_to_base64(plaintext.as_bytes(), recipient).await?;
    let mut modes = BTreeMap::new();
    modes.insert(
        mode.to_string(),
        CachedMode {
            input_hash: input_hash.to_string(),
            keys: values.keys().cloned().collect(),
            data,
        },
    );
    let cache = CacheFile {
        version: FORMAT_VERSION,
        project_root: workspace_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_string_lossy()
            .into_owned(),
        modes,
    };
    let contents = toml::to_string(&cache)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    atomic_write(path, contents.as_bytes()).await
}

async fn run_command(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_process_env: bool,
) -> Result<()> {
    let (program, args) = command
        .split_first()
        .context("a command is required after --")?;
    let mut child = Command::new(program);
    child.args(args);
    for (key, value) in values {
        if override_process_env || std::env::var_os(key).is_none() {
            child.env(key, value);
        }
    }
    let status = child
        .status()
        .await
        .with_context(|| format!("running {}", program.to_string_lossy()))?;
    if status.success() {
        return Ok(());
    }
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::exit(128 + status.signal().unwrap_or(1));
    }
    #[cfg(not(unix))]
    std::process::exit(1);
}

fn validate_env_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("environment variable name must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        bail!("invalid environment variable name: {key}");
    }
    Ok(())
}

fn absolute_from_current(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("reading current directory")?
            .join(path))
    }
}

async fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(".shine-write-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp, contents)
        .await
        .with_context(|| format!("writing {}", temp.display()))?;
    #[cfg(windows)]
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    if let Err(error) = tokio::fs::rename(&temp, path).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_vite_style_layers_in_declared_order() {
        let workspace = Path::new("/tmp/project/shine.workspace.toml");
        let files = vec![
            ".env.shine.toml".into(),
            ".env.local.shine.toml".into(),
            ".env.{mode}.shine.toml".into(),
            ".env.{mode}.local.shine.toml".into(),
        ];
        assert_eq!(
            resolve_sources(workspace, &files, "production").unwrap(),
            vec![
                PathBuf::from("/tmp/project/.env.shine.toml"),
                PathBuf::from("/tmp/project/.env.local.shine.toml"),
                PathBuf::from("/tmp/project/.env.production.shine.toml"),
                PathBuf::from("/tmp/project/.env.production.local.shine.toml"),
            ]
        );
    }

    #[test]
    fn source_rejects_duplicate_plain_and_secret_keys() {
        let error = parse_source(
            Path::new(".env.shine.toml"),
            "version = 1\n[plain]\nTOKEN = \"plain\"\n[secret]\nTOKEN = true\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("both [plain] and [secret]"));
    }

    #[test]
    fn recipient_priority_is_cli_workspace_global() {
        assert_eq!(
            resolve_seal_recipient(Some("cli"), Some("workspace"), Some("global")),
            Some("cli")
        );
        assert_eq!(
            resolve_seal_recipient(None, Some("workspace"), Some("global")),
            Some("workspace")
        );
        assert_eq!(
            resolve_seal_recipient(None, None, Some("global")),
            Some("global")
        );
    }

    #[tokio::test]
    async fn plain_sources_merge_in_declared_order() {
        let directory =
            std::env::temp_dir().join(format!("shine-workspace-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let base = directory.join("base.toml");
        let local = directory.join("local.toml");
        tokio::fs::write(&base, "version = 1\n[plain]\nA = \"base\"\nB = \"base\"\n")
            .await
            .unwrap();
        tokio::fs::write(&local, "version = 1\n[plain]\nB = \"local\"\n")
            .await
            .unwrap();

        let values = compile_sources(&[base, local]).await.unwrap();
        assert_eq!(values.get("A").map(String::as_str), Some("base"));
        assert_eq!(values.get("B").map(String::as_str), Some("local"));
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn plain_only_source_can_be_sealed_without_recipient() {
        let directory = std::env::temp_dir().join(format!("shine-seal-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("env.toml");
        tokio::fs::write(&path, "version = 1\n[plain]\nNAME = \"shine\"\n")
            .await
            .unwrap();

        seal_file(&path, None).await.unwrap();
        let source = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(source.contains("[payload]"));
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_command_injects_workspace_values() {
        let values = BTreeMap::from([("SHINE_RUN_TEST".to_string(), "injected".to_string())]);
        run_command(
            &[
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from("test \"$SHINE_RUN_TEST\" = injected"),
            ],
            &values,
            true,
        )
        .await
        .unwrap();
    }
}
