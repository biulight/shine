//! Sealing for workspace environments.
use super::{
    BTreeSet, BackendKind, Config, Context, DocumentMut, EncryptRecipients, Encryption, Password,
    Path, PathBuf, Result, SECRET_PAYLOAD_VERSION, SecretPayload, SecretState, SensitiveValues,
    Workspace, Zeroizing, absolute_from_current, bail, capture_sources, clean_recipients,
    decrypt_source_payload, find_workspace_optional, local_cache, parse_source, parse_workspace,
    resolve_sources, secret, value,
};

pub async fn handle_seal(
    config: &Config,
    workspace_arg: Option<&Path>,
    file: Option<&Path>,
    backend_arg: Option<&str>,
    recipients_arg: &[String],
) -> Result<()> {
    let workspace_path = find_workspace_optional(workspace_arg).await?;
    let local_config = match &workspace_path {
        Some(path) => local_cache::load_config(config, path).await?,
        None => config.clone(),
    };
    let config = &local_config;
    let lock_scope = workspace_path
        .as_deref()
        .or(file)
        .context("workspace or source file required")?;
    let _lock = SealLock::acquire(lock_scope)?;
    let workspace_snapshot = match &workspace_path {
        Some(path) => Some(tokio::fs::read_to_string(path).await?),
        None => None,
    };
    let workspace = workspace_path
        .as_ref()
        .zip(workspace_snapshot.as_ref())
        .map(|(path, text)| parse_workspace(path, text))
        .transpose()?;
    let mut encryption = resolve_seal_encryption(
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

    let mut source_locks = Vec::new();
    let scope = std::fs::canonicalize(lock_scope)?;
    let mut locked_paths = BTreeSet::new();
    for path in &files {
        validate_seal_source(path).await?;
        let canonical = std::fs::canonicalize(path)?;
        if canonical != scope && locked_paths.insert(canonical.clone()) {
            source_locks.push(SealLock::acquire(&canonical)?);
        }
    }
    let captured = capture_sources(&files).await?;
    let mut contains_secrets = false;
    for (path, contents) in &captured {
        let source = parse_source(
            path,
            contents
                .as_deref()
                .context("source disappeared before sealing")?,
        )?;
        contains_secrets |= !source.secret.is_empty();
    }
    if let Some(EncryptRecipients::Hybrid(recipients)) = &mut encryption {
        recipients.prepare().await?;
    } else if workspace
        .as_ref()
        .and_then(|w| w.env.encryption.backend.as_deref())
        == Some("hybrid")
    {
        eprintln!("This seal explicitly targets only the selected single-backend recipient group.");
    }
    if contains_secrets && let Some(EncryptRecipients::Age(recipients)) = &encryption {
        secret::preflight_age_recipients(recipients).await?;
    }
    for (completed, (path, contents)) in captured.iter().enumerate() {
        let result = async {
            if let Some((path, original)) =
                workspace_path.as_deref().zip(workspace_snapshot.as_deref())
            {
                verify_snapshot(path, original).await?;
            }
            let contents = contents
                .as_deref()
                .context("source disappeared before sealing")?;
            let updated = prepare_sealed_file(path, contents, config, encryption.as_ref()).await?;
            replace_sealed_file(
                path,
                updated.as_bytes(),
                contents,
                workspace_path.as_deref().zip(workspace_snapshot.as_deref()),
            )
            .await
        }
        .await;
        result.with_context(|| format!("sealing stopped: {completed} completed, current file {} not updated, {} unprocessed; retry after resolving the error", path.display(), captured.len() - completed - 1))?;
        println!("sealed {}", path.display());
    }
    Ok(())
}

/// Resolve the backend + recipients to encrypt with for `seal`/`run`, in
/// CLI > workspace `env.encryption` > config precedence. Returns `None` when
/// nothing is configured anywhere, so sealing secretless files never
/// requires a recipient.
pub(super) fn resolve_seal_encryption(
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

    if backend == BackendKind::Hybrid {
        if !cli_recipients.is_empty() {
            bail!("hybrid rejects --recipient; edit both workspace recipient lists");
        }
        let workspace = workspace_encryption.context("hybrid requires workspace access lists")?;
        return Ok(Some(EncryptRecipients::Hybrid(
            secret::hybrid::Recipients::new(
                clean_recipients(&workspace.gpg_recipients),
                clean_recipients(&workspace.age_recipients),
            )?,
        )));
    }
    let cli_recipients = clean_recipients(cli_recipients);
    if !cli_recipients.is_empty() {
        return Ok(Some(match backend {
            BackendKind::Gpg => EncryptRecipients::Gpg(cli_recipients),
            BackendKind::Age => EncryptRecipients::Age(cli_recipients),
            BackendKind::Hybrid => unreachable!(),
        }));
    }

    match backend {
        BackendKind::Hybrid => unreachable!(),
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

pub(super) fn resolve_backend(
    cli_backend: Option<&str>,
    workspace_backend: Option<&str>,
    config_backend: Option<&str>,
) -> Result<BackendKind> {
    if config_backend.is_some_and(|value| value.trim().eq_ignore_ascii_case("hybrid")) {
        bail!("global secret_backend cannot be hybrid; use workspace access lists");
    }
    for candidate in [cli_backend, workspace_backend, config_backend] {
        if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            return value.parse();
        }
    }
    Ok(BackendKind::default())
}

pub(super) async fn existing_workspace_sources(
    path: &Path,
    workspace: &Workspace,
) -> Result<Vec<PathBuf>> {
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

#[cfg(test)]
pub(super) async fn seal_file(
    path: &Path,
    config: &Config,
    encryption: Option<&EncryptRecipients>,
) -> Result<()> {
    validate_seal_source(path).await?;
    let contents = tokio::fs::read_to_string(path).await?;
    let updated = prepare_sealed_file(path, &contents, config, encryption).await?;
    verify_snapshot(path, &contents).await?;
    replace_sealed_file(path, updated.as_bytes(), &contents, None).await
}

pub(super) async fn prepare_sealed_file(
    path: &Path,
    contents: &str,
    config: &Config,
    encryption: Option<&EncryptRecipients>,
) -> Result<String> {
    let source = parse_source(path, contents)?;
    let mut old_values = SecretPayload {
        version: SECRET_PAYLOAD_VERSION,
        values: decrypt_source_payload(path, &source, config).await?,
    };
    let mut new_values = SecretPayload {
        version: SECRET_PAYLOAD_VERSION,
        values: SensitiveValues::default(),
    };

    for (key, state) in &source.secret {
        crate::env::validate_env_key(key)?;
        let secret = match state {
            SecretState::Sealed(true) => old_values
                .values
                .remove(key)
                .with_context(|| format!("{key} is marked sealed but is missing from payload"))?,
            SecretState::Sealed(false) => Password::new()
                .with_prompt(format!("Enter {key}"))
                .with_confirmation("Confirm value", "Values did not match")
                .interact()
                .with_context(|| format!("reading {key}"))?,
            SecretState::Plain(value) => value.clone(),
        };
        new_values.values.insert(key.clone(), secret);
    }

    let encoded = if new_values.values.is_empty() {
        String::new()
    } else {
        let encryption = encryption.context(
            "recipients are required; pass --recipient/--backend, set env.encryption in shine.workspace.toml, or set gpg_recipients/age_recipients",
        )?;
        let plaintext = Zeroizing::new(toml::to_string(&new_values)?);
        secret::encrypt_secret(plaintext.as_bytes(), encryption).await?
    };

    let mut document = contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("invalid source syntax for update"))?;
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
    Ok(document.to_string())
}

pub(super) struct SealLock(std::fs::File);

impl SealLock {
    pub(super) fn acquire(scope: &Path) -> Result<Self> {
        use fs2::FileExt;
        let scope = std::fs::canonicalize(scope).context("resolving seal lock scope")?;
        // Preserve the extension: env.dev and env.prod are distinct lock scopes.
        let mut path = scope.into_os_string();
        path.push(".shine-seal.lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path).context("opening seal lock")?;
        file.try_lock_exclusive()
            .context("another Shine seal is active; retry when it finishes")?;
        Ok(Self(file))
    }
}

impl Drop for SealLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub(super) async fn validate_seal_source(path: &Path) -> Result<()> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .with_context(|| format!("inspecting environment source {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "sealing requires a regular environment source file, not a symbolic link: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            bail!(
                "sealing requires an environment source file with one hard link: {}",
                path.display()
            );
        }
    }
    Ok(())
}

pub(super) async fn verify_snapshot(path: &Path, original: &str) -> Result<()> {
    let current = tokio::fs::read(path)
        .await
        .context("cannot recheck seal snapshot; source not updated")?;
    if current != original.as_bytes() {
        bail!("seal snapshot changed; source not updated; retry");
    }
    Ok(())
}

pub(super) async fn replace_sealed_file(
    path: &Path,
    contents: &[u8],
    original: &str,
    workspace: Option<(&Path, &str)>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    validate_seal_source(path).await?;
    let temp = path.with_extension(format!("shine-seal-{}", uuid::Uuid::new_v4()));
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp).await?;
        file.write_all(contents).await?;
        file.sync_all().await?;
        drop(file);
        if let Some((path, original)) = workspace {
            verify_snapshot(path, original).await?;
        }
        validate_seal_source(path).await?;
        verify_snapshot(path, original).await?;
        // std/tokio rename uses replacing MoveFileExW on Windows. Never unlink the destination.
        tokio::fs::rename(&temp, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result.context("replacing sealed source")
}
