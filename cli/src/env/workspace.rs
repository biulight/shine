use super::broker::{SourceSnapshot, WorkspaceSnapshot};
use crate::commands::EnvWorkspaceExportFormat;
use crate::persist::{atomic_write, atomic_write_private};
use crate::secret::{BackendKind, EncryptRecipients};
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
use zeroize::Zeroize;

const WORKSPACE_FILE: &str = "shine.workspace.toml";
const WORKSPACE_FORMAT_VERSION: u32 = 2;
const ENV_SOURCE_FORMAT_VERSION: u32 = 1;
const SECRET_PAYLOAD_VERSION: u32 = 1;
const CACHE_FORMAT_VERSION: u32 = 1;

/// Initialize a workspace by copying conventional dotenv sources into Shine's
/// explicit TOML source format. The original dotenv files are never modified.
pub async fn handle_init_from_dotenv(
    from_dotenv: bool,
    requested_modes: &[String],
    secrets: &[String],
    force: bool,
    dry_run: bool,
) -> Result<()> {
    if !from_dotenv {
        bail!("pass --from-dotenv to initialize from conventional dotenv files");
    }

    let root = std::env::current_dir().context("resolving current directory")?;
    init_from_dotenv_at(&root, requested_modes, secrets, force, dry_run).await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_export(
    config: &Config,
    format: EnvWorkspaceExportFormat,
    workspace_arg: Option<&Path>,
    mode: &str,
    output: &Path,
    include_secrets: bool,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    validate_mode(mode)?;
    let workspace_path = find_workspace_optional(workspace_arg)
        .await?
        .context("shine.workspace.toml was not found; pass --workspace")?;
    let workspace = load_workspace(&workspace_path).await?;
    let sources = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
    let output = absolute_from_current(output)?;
    if output.exists() && !force {
        bail!(
            "{} already exists; rerun with --force to replace it",
            output.display()
        );
    }

    let mut values = compile_export_sources(&sources, config, include_secrets).await?;
    let mut contents = match format {
        EnvWorkspaceExportFormat::Dotenv => render_dotenv(&values)?,
    };
    if dry_run {
        println!(
            "Would export {} variables for mode {mode} to {}{}",
            values.len(),
            output.display(),
            if include_secrets {
                " (including secrets)"
            } else {
                ""
            }
        );
        for value in values.values_mut() {
            value.zeroize();
        }
        contents.zeroize();
    } else {
        let write_result = if include_secrets {
            atomic_write_private(&output, contents.as_bytes()).await
        } else {
            atomic_write(&output, contents.as_bytes()).await
        };
        for value in values.values_mut() {
            value.zeroize();
        }
        contents.zeroize();
        write_result?;
        println!(
            "Exported {} variables for mode {mode} to {}{}",
            values.len(),
            output.display(),
            if include_secrets {
                " (including secrets)"
            } else {
                ""
            }
        );
        if include_secrets {
            eprintln!("Warning: the exported file contains plaintext secrets; do not commit it.");
        }
    }
    Ok(())
}

fn render_dotenv(values: &BTreeMap<String, String>) -> Result<String> {
    let mut rendered = String::new();
    for (key, value) in values {
        if value.contains('\0') {
            bail!("{key} contains a NUL byte and cannot be represented in dotenv format");
        }
        rendered.push_str(key);
        rendered.push('=');
        rendered.push('"');
        for ch in value.chars() {
            match ch {
                '\\' => rendered.push_str("\\\\"),
                '"' => rendered.push_str("\\\""),
                '\n' => rendered.push_str("\\n"),
                '\r' => rendered.push_str("\\r"),
                _ => rendered.push(ch),
            }
        }
        rendered.push_str("\"\n");
    }
    Ok(rendered)
}

async fn init_from_dotenv_at(
    root: &Path,
    requested_modes: &[String],
    secrets: &[String],
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let modes = dotenv_modes(root, requested_modes)?;
    let sources = dotenv_sources(root, &modes);
    let mut planned = Vec::new();
    let requested_secrets: BTreeSet<_> = secrets.iter().cloned().collect();
    for key in &requested_secrets {
        super::validate_env_key(key)?;
    }
    let mut seen_keys = BTreeSet::new();

    for (input, output) in sources {
        if !input.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(&input)
            .await
            .with_context(|| format!("reading {}", input.display()))?;
        let values = parse_dotenv(&input, &contents)?;
        seen_keys.extend(values.keys().cloned());
        planned.push((output, render_source(&input, &values, &requested_secrets)));
    }
    if planned.is_empty() {
        bail!("no dotenv files found; expected .env, .env.local, or .env.<mode>");
    }
    for key in &requested_secrets {
        if !seen_keys.contains(key) {
            bail!("--secret {key} was not found in an imported dotenv file");
        }
    }

    let workspace = root.join(WORKSPACE_FILE);
    planned.push((workspace, render_workspace(&modes)));
    for (path, _) in &planned {
        if path.exists() && !force {
            bail!(
                "{} already exists; rerun with --force to replace generated files",
                path.display()
            );
        }
    }

    for (path, contents) in &planned {
        let display = path.strip_prefix(root).unwrap_or(path).display();
        if dry_run {
            println!("Would create {display}");
        } else {
            atomic_write(path, contents.as_bytes()).await?;
            println!("Created {display}");
        }
    }
    if !requested_secrets.is_empty() {
        println!("Run `shine env secret seal` after configuring an encryption recipient.");
    }
    Ok(())
}

fn dotenv_modes(root: &Path, requested: &[String]) -> Result<Vec<String>> {
    let mut modes: BTreeSet<String> = requested.iter().cloned().collect();
    for mode in &modes {
        validate_mode(mode)?;
    }
    if modes.is_empty() {
        for entry in
            std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
        {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            let Some(suffix) = name.strip_prefix(".env.") else {
                continue;
            };
            if suffix == "local" || suffix.ends_with(".shine.toml") {
                continue;
            }
            let mode = suffix.strip_suffix(".local").unwrap_or(suffix);
            if mode.is_empty() || mode.contains('.') {
                continue;
            }
            validate_mode(mode)?;
            modes.insert(mode.to_owned());
        }
    }
    if modes.is_empty() {
        modes.insert("development".to_owned());
    }
    Ok(modes.into_iter().collect())
}

fn dotenv_sources(root: &Path, modes: &[String]) -> Vec<(PathBuf, PathBuf)> {
    let mut files = vec![
        (root.join(".env"), root.join(".env.shine.toml")),
        (root.join(".env.local"), root.join(".env.local.shine.toml")),
    ];
    for mode in modes {
        files.push((
            root.join(format!(".env.{mode}")),
            root.join(format!(".env.{mode}.shine.toml")),
        ));
        files.push((
            root.join(format!(".env.{mode}.local")),
            root.join(format!(".env.{mode}.local.shine.toml")),
        ));
    }
    files
}

fn render_workspace(modes: &[String]) -> String {
    let default_mode = &modes[0];
    let rendered_modes = modes
        .iter()
        .map(|mode| format!("\"{mode}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# Managed by `shine env workspace init --from-dotenv`.\n\
         # Edit the source files below; later files override earlier ones.\n\
         version = {WORKSPACE_FORMAT_VERSION}\n\n\
         [env]\n\
         # Run with: shine env run --mode {default_mode} -- <command>\n\
         modes = [{modes}]\n\
         default_mode = \"{default_mode}\"\n\
         files = [\n\
           \".env.shine.toml\", # shared defaults\n\
           \".env.local.shine.toml\", # local-only overrides; do not commit\n\
           \".env.{{mode}}.shine.toml\", # mode-specific values\n\
           \".env.{{mode}}.local.shine.toml\", # local mode overrides; do not commit\n\
         ]\n\n\
         # Add GPG recipients before sealing values in [secret].\n\
         # [env.encryption]\n\
         # gpg_recipients = [\"alice@example.com\", \"bob@example.com\"]\n",
        modes = rendered_modes,
    )
}

fn render_source(
    input: &Path,
    values: &BTreeMap<String, String>,
    secrets: &BTreeSet<String>,
) -> String {
    let mut document = DocumentMut::new();
    document["version"] = value(ENV_SOURCE_FORMAT_VERSION as i64);
    let mut plain = toml_edit::Table::new();
    let mut secret = toml_edit::Table::new();
    for (key, value_text) in values {
        if secrets.contains(key) {
            secret[key] = value(value_text);
        } else {
            plain[key] = value(value_text);
        }
    }
    if !plain.is_empty() {
        document["plain"] = toml_edit::Item::Table(plain);
    }
    if !secret.is_empty() {
        document["secret"] = toml_edit::Item::Table(secret);
    }
    let source_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dotenv file");
    let mut contents = format!(
        "# Imported from {source_name}. Keep non-sensitive values in [plain].\n\
         # Move sensitive values to [secret], then run `shine env secret seal`.\n"
    );
    contents.push_str(&document.to_string());
    if secrets.is_empty() {
        contents.push_str(
            "\n# Optional: move sensitive values here, then run `shine env secret seal`.\n[secret]\n",
        );
    }
    contents
}

fn parse_dotenv(path: &Path, contents: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, raw_value)) = line.split_once('=') else {
            bail!(
                "{}:{} is not a KEY=VALUE dotenv entry",
                path.display(),
                index + 1
            );
        };
        let key = key.trim();
        super::validate_env_key(key)
            .with_context(|| format!("{}:{}", path.display(), index + 1))?;
        let value_text = parse_dotenv_value(path, index + 1, raw_value)?;
        values.insert(key.to_owned(), value_text);
    }
    Ok(values)
}

fn parse_dotenv_value(path: &Path, line: usize, raw: &str) -> Result<String> {
    let raw = raw.trim();
    let value = if let Some(value) = raw.strip_prefix('\'') {
        parse_quoted_dotenv_value(value, '\'', "single")?
    } else if let Some(value) = raw.strip_prefix('"') {
        let value = parse_quoted_dotenv_value(value, '"', "double")?;
        if value.contains('\\') {
            bail!(
                "{}:{line} uses escaped double-quoted dotenv content; resolve it before importing",
                path.display()
            );
        }
        value
    } else {
        raw.split_once(" #")
            .map(|(value, _)| value)
            .unwrap_or(raw)
            .trim_end()
    };
    if value.contains("${") {
        bail!(
            "{}:{line} uses dotenv interpolation; resolve it before importing",
            path.display()
        );
    }
    Ok(value.to_owned())
}

fn parse_quoted_dotenv_value<'a>(raw: &'a str, quote: char, style: &str) -> Result<&'a str> {
    let closing = raw
        .find(quote)
        .with_context(|| format!("unterminated {style}-quoted dotenv value"))?;
    let trailing = raw[closing + quote.len_utf8()..].trim_start();
    if !trailing.is_empty() && !trailing.starts_with('#') {
        bail!("unexpected content after {style}-quoted dotenv value");
    }
    Ok(&raw[..closing])
}

#[derive(Clone, Debug, Deserialize)]
pub struct Workspace {
    #[serde(default = "workspace_format_version")]
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
    #[serde(rename = "recipient")]
    legacy_recipient: Option<String>,
    #[serde(default)]
    gpg_recipients: Vec<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    age_recipients: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SourceFile {
    #[serde(default = "env_source_format_version")]
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

fn workspace_format_version() -> u32 {
    WORKSPACE_FORMAT_VERSION
}

fn env_source_format_version() -> u32 {
    ENV_SOURCE_FORMAT_VERSION
}

pub async fn handle_seal(
    config: &Config,
    workspace_arg: Option<&Path>,
    file: Option<&Path>,
    backend_arg: Option<&str>,
    recipients_arg: &[String],
) -> Result<()> {
    let workspace_path = find_workspace_optional(workspace_arg).await?;
    let workspace = match &workspace_path {
        Some(path) => Some(load_workspace(path).await?),
        None => None,
    };
    let encryption = resolve_seal_encryption(
        backend_arg,
        recipients_arg,
        workspace
            .as_ref()
            .map(|workspace| &workspace.env.encryption),
        config,
    )?;

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
        seal_file(path, config, encryption.as_ref()).await?;
        println!("sealed {}", path.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Command handler keeps independent Clap options explicit.
pub async fn handle_run(
    config: &Config,
    workspace_arg: Option<&Path>,
    mode_arg: Option<&str>,
    no_workspace: bool,
    with: &[String],
    secret_broker: bool,
    broker_secrets: &[String],
    command: &[OsString],
) -> Result<()> {
    let explicit = resolve_explicit_values(config, with).await?;
    if !secret_broker && !broker_secrets.is_empty() {
        bail!("--secret requires --secret-broker");
    }
    if secret_broker {
        let argv = broker_command_argv(command)?;
        if no_workspace {
            if broker_secrets.is_empty() {
                bail!("--no-workspace --secret-broker requires at least one --secret");
            }
            let values = crate::ssh::request_direct_secrets(broker_secrets, &argv).await?;
            return run_broker_command(
                command,
                BTreeMap::new(),
                false,
                merge_explicit(explicit, values)?,
            )
            .await;
        }
        if !broker_secrets.is_empty() {
            bail!(
                "workspace broker requests derive release keys from policy; do not pass --secret"
            );
        }
        let mode = mode_arg.context("workspace --secret-broker requires --mode")?;
        let snapshot = snapshot_for_broker(workspace_arg, mode).await?;
        let mut values = plain_values_from_broker_snapshot(&snapshot)?;
        let secrets = crate::ssh::request_workspace_secrets(snapshot.clone(), &argv).await?;
        values.extend(secrets);
        return run_broker_command(command, values, snapshot.override_process_env, explicit).await;
    }
    // `--no-workspace` disables discovery entirely: only explicit `--with` values
    // and the inherited process environment reach the command. Generated Bun
    // launchers rely on this so a nearby shine.workspace.toml can never hijack them.
    let workspace_path = if no_workspace {
        None
    } else {
        find_workspace_optional(workspace_arg).await?
    };
    let (values, override_process_env) = if let Some(workspace_path) = workspace_path {
        let workspace = load_workspace(&workspace_path).await?;
        let mode = mode_arg
            .or(workspace.env.default_mode.as_deref())
            .context("environment mode is required; pass --mode or set env.default_mode")?;
        validate_mode(mode)?;
        let sources = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
        let input_hash = calculate_input_hash(&workspace_path, mode, &sources).await?;
        let encryption =
            resolve_seal_encryption(None, &[], Some(&workspace.env.encryption), config)?;
        let cache_path = cache_path(&workspace_path, mode)?;
        let values = match read_valid_cache(&cache_path, mode, &input_hash, config).await {
            Ok(Some(values)) => values,
            Ok(None) => {
                let values = compile_sources(&sources, config).await?;
                if let Some(encryption) = &encryption
                    && let Err(error) = write_cache(
                        &cache_path,
                        &workspace_path,
                        mode,
                        &input_hash,
                        &values,
                        encryption,
                    )
                    .await
                {
                    eprintln!("Warning: could not update environment cache: {error:#}");
                }
                values
            }
            Err(error) => {
                eprintln!("Warning: ignoring unreadable environment cache: {error:#}");
                compile_sources(&sources, config).await?
            }
        };
        (values, workspace.env.override_process_env)
    } else {
        if !no_workspace && explicit.is_empty() {
            bail!("shine.workspace.toml was not found; pass --workspace or --no-workspace");
        }
        if mode_arg.is_some() {
            bail!("--mode requires a shine.workspace.toml");
        }
        (BTreeMap::new(), false)
    };

    run_command(command, &values, override_process_env, &explicit).await
}

fn broker_command_argv(command: &[OsString]) -> Result<Vec<String>> {
    command
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_string)
                .context("secret broker command arguments must be valid UTF-8")
        })
        .collect()
}

fn merge_explicit(
    mut explicit: BTreeMap<String, String>,
    broker: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    for (key, value) in broker {
        if explicit.insert(key.clone(), value).is_some() {
            bail!("broker target {key} conflicts with an explicit --with target");
        }
    }
    Ok(explicit)
}

async fn resolve_explicit_values(
    config: &Config,
    specs: &[String],
) -> Result<BTreeMap<String, String>> {
    let parsed = super::parse_env_specs(specs)?;

    let env = super::EnvConfig::load_or_init(config).await?;
    let mut values = BTreeMap::new();
    for spec in parsed {
        let value = match super::resolve_stored_value(&env, &spec.source)? {
            super::StoredValue::Secret {
                key: secret_key,
                value: ciphertext,
            } => secret::decrypt_secret(ciphertext, &config.resolved_age_identities())
                .await
                .with_context(|| format!("decrypting {secret_key}"))?,
            super::StoredValue::Plaintext(value) => value.to_string(),
        };
        values.insert(spec.target, value);
    }
    Ok(values)
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

async fn load_workspace(path: &Path) -> Result<Workspace> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    parse_workspace(path, &contents)
}

fn parse_workspace(path: &Path, contents: &str) -> Result<Workspace> {
    let workspace: Workspace =
        toml::from_str(contents).with_context(|| format!("parsing {}", path.display()))?;
    if workspace.version < WORKSPACE_FORMAT_VERSION {
        bail!(
            "workspace version {} in {} is retired; run `shine state migrate`",
            workspace.version,
            path.display()
        );
    }
    if workspace.version != WORKSPACE_FORMAT_VERSION {
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
    if workspace.env.files.is_empty() {
        bail!("env.files must contain at least one source path");
    }
    Ok(workspace)
}

/// Resolve the backend + recipients to encrypt with for `seal`/`run`, in
/// CLI > workspace `env.encryption` > config precedence. Returns `None` when
/// nothing is configured anywhere, so sealing secretless files never
/// requires a recipient.
fn resolve_seal_encryption(
    cli_backend: Option<&str>,
    cli_recipients: &[String],
    workspace_encryption: Option<&Encryption>,
    config: &Config,
) -> Result<Option<EncryptRecipients>> {
    let backend = resolve_backend(
        cli_backend,
        workspace_encryption.and_then(|encryption| encryption.backend.as_deref()),
        config.secret_backend.as_deref(),
    )?;

    let cli_recipients = clean_recipients(cli_recipients);
    if !cli_recipients.is_empty() {
        return Ok(Some(match backend {
            BackendKind::Gpg => EncryptRecipients::Gpg(cli_recipients),
            BackendKind::Age => EncryptRecipients::Age(cli_recipients),
        }));
    }

    match backend {
        BackendKind::Gpg => {
            let workspace_recipients = workspace_encryption
                .map(|encryption| clean_recipients(&encryption.gpg_recipients))
                .unwrap_or_default();
            let recipients = if !workspace_recipients.is_empty() {
                workspace_recipients
            } else {
                clean_recipients(&config.gpg_recipients)
            };
            Ok((!recipients.is_empty()).then_some(EncryptRecipients::Gpg(recipients)))
        }
        BackendKind::Age => {
            let workspace_recipients = workspace_encryption
                .map(|encryption| clean_recipients(&encryption.age_recipients))
                .unwrap_or_default();
            let recipients = if !workspace_recipients.is_empty() {
                workspace_recipients
            } else {
                clean_recipients(&config.age_recipients)
            };
            Ok((!recipients.is_empty()).then_some(EncryptRecipients::Age(recipients)))
        }
    }
}

fn resolve_backend(
    cli_backend: Option<&str>,
    workspace_backend: Option<&str>,
    config_backend: Option<&str>,
) -> Result<BackendKind> {
    for candidate in [cli_backend, workspace_backend, config_backend] {
        if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            return value.parse();
        }
    }
    Ok(BackendKind::default())
}

fn clean_recipients(recipients: &[String]) -> Vec<String> {
    recipients
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
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

pub(crate) fn validate_broker_mode(mode: &str) -> Result<()> {
    validate_mode(mode)
}

/// Reads one workspace/mode exactly once for SSH broker hashing and execution.
/// The returned bytes are retained by the remote runner until the child starts,
/// so a successful authorization never re-reads mutable files.
pub async fn snapshot_for_broker(
    workspace_arg: Option<&Path>,
    mode: &str,
) -> Result<WorkspaceSnapshot> {
    validate_mode(mode)?;
    let workspace_path = find_workspace_optional(workspace_arg)
        .await?
        .context("shine.workspace.toml was not found; pass --workspace")?;
    let workspace_contents = tokio::fs::read_to_string(&workspace_path)
        .await
        .with_context(|| format!("reading {}", workspace_path.display()))?;
    let workspace = parse_workspace(&workspace_path, &workspace_contents)?;
    let source_paths = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    let mut sources = Vec::new();
    for source_path in source_paths {
        if !source_path.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(&source_path)
            .await
            .with_context(|| format!("reading {}", source_path.display()))?;
        // Parse now so malformed/unsealed metadata never reaches the broker.
        let source = parse_source(&source_path, &contents)?;
        for (key, state) in &source.secret {
            if !matches!(state, SecretState::Sealed(true)) {
                bail!(
                    "{key} in {} is not sealed; run `shine env secret seal`",
                    source_path.display()
                );
            }
        }
        let display_path = source_path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| source_path.clone())
            .to_string_lossy()
            .into_owned();
        sources.push(SourceSnapshot {
            path: display_path,
            contents,
        });
    }
    if sources.is_empty() {
        bail!("none of the configured environment source files exist");
    }
    Ok(WorkspaceSnapshot {
        workspace_path: workspace_path.to_string_lossy().into_owned(),
        workspace_contents,
        mode: mode.to_string(),
        override_process_env: workspace.env.override_process_env,
        sources,
    })
}

pub(crate) fn declared_secrets_from_source(path: &str, contents: &str) -> Result<Vec<String>> {
    let source = parse_source(Path::new(path), contents)?;
    let mut keys = source.secret.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    Ok(keys)
}

pub fn plain_values_from_broker_snapshot(
    snapshot: &WorkspaceSnapshot,
) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for source in &snapshot.sources {
        let parsed = parse_source(Path::new(&source.path), &source.contents)?;
        values.extend(parsed.plain);
    }
    Ok(values)
}

pub async fn decrypt_broker_snapshot(
    config: &Config,
    snapshot: &WorkspaceSnapshot,
    release: &[String],
) -> Result<BTreeMap<String, String>> {
    let release = release.iter().cloned().collect::<BTreeSet<_>>();
    let mut values = BTreeMap::new();
    for source in &snapshot.sources {
        let path = Path::new(&source.path);
        let parsed = parse_source(path, &source.contents)?;
        for (key, state) in &parsed.secret {
            if !matches!(state, SecretState::Sealed(true)) {
                bail!("{key} in {} is not sealed", source.path);
            }
        }
        let secrets = decrypt_source_payload(path, &parsed, config).await?;
        let expected = parsed.secret.keys().cloned().collect::<BTreeSet<_>>();
        let actual = secrets.keys().cloned().collect::<BTreeSet<_>>();
        if expected != actual {
            bail!(
                "secret key list does not match encrypted payload in {}",
                source.path
            );
        }
        values.extend(secrets.into_iter().filter(|(key, _)| release.contains(key)));
    }
    if values.keys().cloned().collect::<BTreeSet<_>>() != release {
        bail!("broker response does not contain every released secret key");
    }
    Ok(values)
}

async fn seal_file(
    path: &Path,
    config: &Config,
    encryption: Option<&EncryptRecipients>,
) -> Result<()> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let source = parse_source(path, &contents)?;
    let mut old_values = decrypt_source_payload(path, &source, config).await?;
    let mut new_values = BTreeMap::new();

    for (key, state) in &source.secret {
        super::validate_env_key(key)?;
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
        let encryption = encryption.context(
            "recipients are required; pass --recipient/--backend, set env.encryption in shine.workspace.toml, or set gpg_recipients/age_recipients",
        )?;
        let plaintext = toml::to_string(&SecretPayload {
            version: SECRET_PAYLOAD_VERSION,
            values: new_values,
        })?;
        secret::encrypt_secret(plaintext.as_bytes(), encryption).await?
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
    if source.version != ENV_SOURCE_FORMAT_VERSION {
        bail!(
            "unsupported environment source version {} in {}",
            source.version,
            path.display()
        );
    }
    for key in source.plain.keys().chain(source.secret.keys()) {
        super::validate_env_key(key)?;
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
    config: &Config,
) -> Result<BTreeMap<String, String>> {
    if source.payload.data.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let plaintext = secret::decrypt_secret(&source.payload.data, &config.resolved_age_identities())
        .await
        .with_context(|| format!("decrypting {}", path.display()))?;
    let payload: SecretPayload = toml::from_str(&plaintext)
        .with_context(|| format!("parsing decrypted payload from {}", path.display()))?;
    if payload.version != SECRET_PAYLOAD_VERSION {
        bail!("unsupported encrypted payload version {}", payload.version);
    }
    Ok(payload.values)
}

async fn load_sealed_source(path: &Path, config: &Config) -> Result<BTreeMap<String, String>> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let source = parse_source(path, &contents)?;
    for (key, state) in &source.secret {
        if !matches!(state, SecretState::Sealed(true)) {
            bail!(
                "{key} in {} is not sealed; run `shine env secret seal`",
                path.display()
            );
        }
    }
    let secrets = decrypt_source_payload(path, &source, config).await?;
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

async fn compile_sources(sources: &[PathBuf], config: &Config) -> Result<BTreeMap<String, String>> {
    let mut merged = BTreeMap::new();
    let mut loaded = 0usize;
    for path in sources {
        if !path.is_file() {
            continue;
        }
        merged.extend(load_sealed_source(path, config).await?);
        loaded += 1;
    }
    if loaded == 0 {
        bail!("none of the configured environment source files exist");
    }
    Ok(merged)
}

async fn compile_export_sources(
    sources: &[PathBuf],
    config: &Config,
    include_secrets: bool,
) -> Result<BTreeMap<String, String>> {
    if include_secrets {
        return compile_sources(sources, config).await;
    }

    let mut merged = BTreeMap::new();
    let mut loaded = 0usize;
    for path in sources {
        if !path.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        let source = parse_source(path, &contents)?;
        for key in source.secret.keys() {
            // A later secret declaration shadows an earlier plain value even
            // when secrets are intentionally omitted from the export.
            merged.remove(key);
        }
        merged.extend(source.plain);
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
    hash.update(CACHE_FORMAT_VERSION.to_le_bytes());
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
    config: &Config,
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
    if cache.version != CACHE_FORMAT_VERSION || cached.input_hash != input_hash {
        return Ok(None);
    }
    let plaintext = secret::decrypt_secret(&cached.data, &config.resolved_age_identities()).await?;
    let payload: SecretPayload = toml::from_str(&plaintext)?;
    let keys: Vec<_> = payload.values.keys().cloned().collect();
    if payload.version != SECRET_PAYLOAD_VERSION || keys != cached.keys {
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
    recipients: &EncryptRecipients,
) -> Result<()> {
    let plaintext = toml::to_string(&SecretPayload {
        version: SECRET_PAYLOAD_VERSION,
        values: values.clone(),
    })?;
    let data = secret::encrypt_secret(plaintext.as_bytes(), recipients).await?;
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
        version: CACHE_FORMAT_VERSION,
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
    explicit: &BTreeMap<String, String>,
) -> Result<()> {
    let status = command_status(command, values, override_process_env, explicit).await?;
    finish_command_status(status)
}

async fn run_broker_command(
    command: &[OsString],
    mut values: BTreeMap<String, String>,
    override_process_env: bool,
    mut explicit: BTreeMap<String, String>,
) -> Result<()> {
    let status = command_status(command, &values, override_process_env, &explicit).await;
    for value in values.values_mut().chain(explicit.values_mut()) {
        value.zeroize();
    }
    finish_command_status(status?)
}

async fn command_status(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_process_env: bool,
    explicit: &BTreeMap<String, String>,
) -> Result<std::process::ExitStatus> {
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
    child.envs(explicit);
    let status = child
        .status()
        .await
        .with_context(|| format!("running {}", program.to_string_lossy()))?;
    Ok(status)
}

fn finish_command_status(status: std::process::ExitStatus) -> Result<()> {
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

fn absolute_from_current(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("reading current directory")?
            .join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_import_parses_common_frontend_entries() {
        let values = parse_dotenv(
            Path::new(".env"),
            "# base\nexport VITE_NAME = \"Shine\" # display name\nVITE_OWNER='Shine team' # owner\nVITE_URL=https://example.test # note\nEMPTY=\n",
        )
        .unwrap();

        assert_eq!(values.get("VITE_NAME").map(String::as_str), Some("Shine"));
        assert_eq!(
            values.get("VITE_OWNER").map(String::as_str),
            Some("Shine team")
        );
        assert_eq!(
            values.get("VITE_URL").map(String::as_str),
            Some("https://example.test")
        );
        assert_eq!(values.get("EMPTY").map(String::as_str), Some(""));
    }

    #[test]
    fn dotenv_import_rejects_interpolation() {
        let error = parse_dotenv(Path::new(".env"), "VITE_URL=${BASE_URL}/api\n").unwrap_err();
        assert!(error.to_string().contains("dotenv interpolation"));
    }

    #[test]
    fn dotenv_export_is_stable_and_escapes_multiline_values() {
        let rendered = render_dotenv(&BTreeMap::from([
            ("ALPHA".to_owned(), "plain".to_owned()),
            (
                "COMPLEX".to_owned(),
                "quote\" slash\\ first\nsecond".to_owned(),
            ),
        ]))
        .unwrap();

        assert_eq!(
            rendered,
            "ALPHA=\"plain\"\nCOMPLEX=\"quote\\\" slash\\\\ first\\nsecond\"\n"
        );
    }

    #[test]
    fn dotenv_export_rejects_nul_values() {
        let error = render_dotenv(&BTreeMap::from([(
            "BROKEN".to_owned(),
            "before\0after".to_owned(),
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("NUL byte"));
    }

    #[test]
    fn dotenv_mode_discovery_ignores_generated_sources() {
        let directory = std::env::temp_dir().join(format!("shine-dotenv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(".env.development"), "VITE_A=1\n").unwrap();
        std::fs::write(directory.join(".env.production.local"), "VITE_A=2\n").unwrap();
        std::fs::write(directory.join(".env.development.shine.toml"), "version=1\n").unwrap();

        assert_eq!(
            dotenv_modes(&directory, &[]).unwrap(),
            vec!["development", "production"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rendered_source_marks_only_requested_keys_secret() {
        let source = render_source(
            Path::new(".env"),
            &BTreeMap::from([
                ("PUBLIC".to_owned(), "yes".to_owned()),
                ("TOKEN".to_owned(), "secret".to_owned()),
            ]),
            &BTreeSet::from(["TOKEN".to_owned()]),
        );
        let parsed: SourceFile = toml::from_str(&source).unwrap();
        assert!(source.contains("Imported from .env"));
        assert_eq!(parsed.plain.get("PUBLIC").map(String::as_str), Some("yes"));
        assert!(
            matches!(parsed.secret.get("TOKEN"), Some(SecretState::Plain(value)) if value == "secret")
        );
    }

    #[test]
    fn rendered_source_includes_an_empty_secret_template() {
        let source = render_source(
            Path::new(".env"),
            &BTreeMap::from([("PUBLIC".to_owned(), "yes".to_owned())]),
            &BTreeSet::new(),
        );
        assert!(source.contains("Optional: move sensitive values"));
        let parsed: SourceFile = toml::from_str(&source).unwrap();
        assert!(parsed.secret.is_empty());
    }

    #[tokio::test]
    async fn dotenv_init_creates_vite_ordered_workspace_without_touching_sources() {
        let directory =
            std::env::temp_dir().join(format!("shine-dotenv-init-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        tokio::fs::write(
            directory.join(".env"),
            "VITE_API=https://api.example.test\nTOKEN=unsealed\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            directory.join(".env.development"),
            "VITE_API=http://localhost:3000\n",
        )
        .await
        .unwrap();

        init_from_dotenv_at(&directory, &[], &["TOKEN".to_owned()], false, false)
            .await
            .unwrap();

        let workspace = tokio::fs::read_to_string(directory.join(WORKSPACE_FILE))
            .await
            .unwrap();
        assert!(
            workspace.find(".env.local.shine.toml").unwrap()
                < workspace.find(".env.{mode}.shine.toml").unwrap()
        );
        assert!(workspace.contains("Managed by `shine env workspace init --from-dotenv`"));
        assert!(workspace.contains("Add GPG recipients"));
        let base = tokio::fs::read_to_string(directory.join(".env.shine.toml"))
            .await
            .unwrap();
        assert!(base.contains("[secret]"));
        assert!(base.contains("TOKEN = \"unsealed\""));
        assert_eq!(
            tokio::fs::read_to_string(directory.join(".env"))
                .await
                .unwrap(),
            "VITE_API=https://api.example.test\nTOKEN=unsealed\n"
        );
        assert!(
            init_from_dotenv_at(&directory, &[], &[], false, false)
                .await
                .is_err()
        );
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

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
    fn seal_encryption_gpg_recipients_priority_is_cli_workspace_config() {
        let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.gpg_recipients = vec!["global-one".to_string(), "global-two".to_string()];
        let workspace_encryption = Encryption {
            legacy_recipient: None,
            gpg_recipients: vec!["workspace-one".to_string(), "workspace-two".to_string()],
            backend: None,
            age_recipients: Vec::new(),
        };

        let cli = resolve_seal_encryption(
            None,
            &["cli".to_string()],
            Some(&workspace_encryption),
            &config,
        )
        .unwrap();
        assert!(matches!(cli, Some(EncryptRecipients::Gpg(values)) if values == ["cli"]));

        let workspace =
            resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
        assert!(
            matches!(workspace, Some(EncryptRecipients::Gpg(values)) if values == ["workspace-one", "workspace-two"])
        );

        let global = resolve_seal_encryption(None, &[], None, &config).unwrap();
        assert!(
            matches!(global, Some(EncryptRecipients::Gpg(values)) if values == ["global-one", "global-two"])
        );
    }

    #[test]
    fn seal_encryption_returns_none_when_nothing_configured() {
        let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&dir);

        assert!(
            resolve_seal_encryption(None, &[], None, &config)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn seal_encryption_age_recipients_prefer_workspace_over_config() {
        let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.secret_backend = Some("age".to_string());
        config.age_recipients = vec!["age1config".to_string()];
        let workspace_encryption = Encryption {
            legacy_recipient: None,
            gpg_recipients: Vec::new(),
            backend: None,
            age_recipients: vec!["age1workspace".to_string()],
        };

        let resolved =
            resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
        assert!(
            matches!(resolved, Some(EncryptRecipients::Age(values)) if values == ["age1workspace"])
        );

        let fallback = resolve_seal_encryption(
            None,
            &[],
            Some(&Encryption {
                legacy_recipient: None,
                gpg_recipients: Vec::new(),
                backend: None,
                age_recipients: Vec::new(),
            }),
            &config,
        )
        .unwrap();
        assert!(
            matches!(fallback, Some(EncryptRecipients::Age(values)) if values == ["age1config"])
        );
    }

    #[test]
    fn seal_encryption_backend_priority_is_cli_workspace_config() {
        let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.secret_backend = Some("age".to_string());
        config.gpg_recipients = vec!["global".to_string()];
        let workspace_encryption = Encryption {
            legacy_recipient: None,
            gpg_recipients: vec!["workspace".to_string()],
            backend: Some("gpg".to_string()),
            age_recipients: Vec::new(),
        };

        let resolved =
            resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
        assert!(matches!(resolved, Some(EncryptRecipients::Gpg(_))));

        let resolved_age = resolve_seal_encryption(None, &[], None, &config).unwrap();
        assert!(
            resolved_age.is_none(),
            "age backend with no age_recipients should be lazily None: {resolved_age:?}"
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

        let config = Config::new_for_test(&directory);
        let values = compile_sources(&[base, local], &config).await.unwrap();
        assert_eq!(values.get("A").map(String::as_str), Some("base"));
        assert_eq!(values.get("B").map(String::as_str), Some("local"));
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_export_plain_is_standalone_and_respects_secret_shadowing() {
        let directory =
            std::env::temp_dir().join(format!("shine-workspace-export-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let workspace_path = directory.join(WORKSPACE_FILE);
        tokio::fs::write(
            &workspace_path,
            "version = 2\n[env]\nmodes = [\"production\"]\nfiles = [\"base.toml\", \"production.toml\"]\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            directory.join("base.toml"),
            "version = 1\n[plain]\nPUBLIC = \"base\"\nSHADOWED = \"old\"\n[secret]\nTOKEN = \"pending\"\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            directory.join("production.toml"),
            "version = 1\n[plain]\nPUBLIC = \"production\"\n[secret]\nSHADOWED = true\n",
        )
        .await
        .unwrap();
        let output = directory.join(".env.production.local");
        let config = Config::new_for_test(&directory);

        handle_export(
            &config,
            EnvWorkspaceExportFormat::Dotenv,
            Some(&workspace_path),
            "production",
            &output,
            false,
            false,
            true,
        )
        .await
        .unwrap();
        assert!(!output.exists());

        handle_export(
            &config,
            EnvWorkspaceExportFormat::Dotenv,
            Some(&workspace_path),
            "production",
            &output,
            false,
            false,
            false,
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&output).await.unwrap(),
            "PUBLIC=\"production\"\n"
        );
        assert!(
            handle_export(
                &config,
                EnvWorkspaceExportFormat::Dotenv,
                Some(&workspace_path),
                "production",
                &output,
                false,
                false,
                false,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("--force")
        );
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_export_requires_explicit_secret_inclusion() {
        let directory =
            std::env::temp_dir().join(format!("shine-workspace-export-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let source = directory.join("source.toml");
        tokio::fs::write(
            &source,
            "version = 1\n[plain]\nPUBLIC = \"safe\"\n[secret]\nTOKEN = \"pending\"\n",
        )
        .await
        .unwrap();
        let config = Config::new_for_test(&directory);

        let plain = compile_export_sources(std::slice::from_ref(&source), &config, false)
            .await
            .unwrap();
        assert_eq!(plain, BTreeMap::from([("PUBLIC".into(), "safe".into())]));

        let error = compile_export_sources(&[source], &config, true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("is not sealed"));
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

        let config = Config::new_for_test(&directory);
        seal_file(&path, &config, None).await.unwrap();
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
            &BTreeMap::new(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn explicit_values_support_aliases_and_multiple_keys() {
        let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&directory);
        config.env.insert("TOKEN_A".into(), "alpha".into());
        config.env.insert("TOKEN_B".into(), "beta".into());

        let values =
            resolve_explicit_values(&config, &["TOKEN_A".into(), "TOKEN_B=OTHER_TOKEN".into()])
                .await
                .unwrap();

        assert_eq!(values.get("TOKEN_A").map(String::as_str), Some("alpha"));
        assert_eq!(values.get("OTHER_TOKEN").map(String::as_str), Some("beta"));
    }

    #[tokio::test]
    async fn explicit_values_reject_duplicate_targets_before_resolution() {
        let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&directory);

        let error =
            resolve_explicit_values(&config, &["TOKEN_A=TOKEN".into(), "TOKEN_B=TOKEN".into()])
                .await
                .unwrap_err();

        assert!(error.to_string().contains("duplicate target variable"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn no_workspace_injects_explicit_without_discovery() {
        let directory = std::env::temp_dir().join(format!("shine-nows-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&directory);
        config.env.insert("SHINE_NOWS_TOKEN".into(), "alpha".into());

        // no_workspace = true must skip discovery entirely and inject only --with.
        handle_run(
            &config,
            None,
            None,
            true,
            &["SHINE_NOWS_TOKEN".into()],
            false,
            &[],
            &[
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from("test \"$SHINE_NOWS_TOKEN\" = alpha"),
            ],
        )
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn no_workspace_allows_empty_with() {
        let directory =
            std::env::temp_dir().join(format!("shine-nows-empty-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&directory);

        handle_run(
            &config,
            None,
            None,
            true,
            &[],
            false,
            &[],
            &[
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from("true"),
            ],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn explicit_values_reject_invalid_or_missing_keys() {
        let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&directory);

        let invalid = resolve_explicit_values(&config, &["BAD-KEY".into()])
            .await
            .unwrap_err();
        assert!(
            invalid
                .to_string()
                .contains("invalid environment variable name")
        );

        let missing = resolve_explicit_values(&config, &["MISSING".into()])
            .await
            .unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("MISSING_SECRET or MISSING is not set")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn explicit_values_override_workspace_and_process_values() {
        let _guard = crate::test_support::env_lock();
        // SAFETY: the shared test env lock serializes process environment mutation.
        unsafe { std::env::set_var("SHINE_RUN_OVERRIDE_TEST", "process") };
        let workspace = BTreeMap::from([(
            "SHINE_RUN_OVERRIDE_TEST".to_string(),
            "workspace".to_string(),
        )]);
        let explicit = BTreeMap::from([(
            "SHINE_RUN_OVERRIDE_TEST".to_string(),
            "explicit".to_string(),
        )]);

        run_command(
            &[
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from("test \"$SHINE_RUN_OVERRIDE_TEST\" = explicit"),
            ],
            &workspace,
            false,
            &explicit,
        )
        .await
        .unwrap();

        assert_eq!(
            std::env::var("SHINE_RUN_OVERRIDE_TEST").as_deref(),
            Ok("process")
        );
        // SAFETY: the shared test env lock serializes process environment mutation.
        unsafe { std::env::remove_var("SHINE_RUN_OVERRIDE_TEST") };
    }
}
