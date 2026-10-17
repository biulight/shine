//! Cache for workspace environments.
use super::{
    BTreeMap, BaseDirs, CACHE_FORMAT_VERSION, CacheFile, CachedMode, Config, Context,
    EncryptRecipients, Path, PathBuf, Result, SECRET_PAYLOAD_VERSION, SecretPayload,
    SensitiveValues, Zeroizing, atomic_write, bail, secret,
};

pub(super) fn cache_path(workspace_path: &Path, mode: &str) -> Result<PathBuf> {
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let project_id = shine_core::digest::sha256_hex(canonical.to_string_lossy().as_bytes());
    let base = BaseDirs::new().context("resolving system cache directory")?;
    Ok(base
        .cache_dir()
        .join("shine")
        .join("projects")
        .join(project_id)
        .join(format!("env-{mode}.toml")))
}

pub(super) async fn read_valid_cache(
    path: &Path,
    mode: &str,
    input_hash: &str,
    config: &Config,
) -> Result<Option<SensitiveValues>> {
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
    let plaintext = Zeroizing::new(secret::decrypt_with_config(&cached.data, config).await?);
    let mut payload: SecretPayload = toml::from_str(&plaintext)
        .map_err(|_| anyhow::anyhow!("invalid encrypted environment cache payload"))?;
    let keys: Vec<_> = payload.values.keys().cloned().collect();
    if payload.version != SECRET_PAYLOAD_VERSION || keys != cached.keys {
        bail!("compiled environment cache failed integrity validation");
    }
    Ok(Some(std::mem::take(&mut payload.values)))
}

pub(super) async fn write_cache(
    path: &Path,
    workspace_path: &Path,
    mode: &str,
    input_hash: &str,
    values: &BTreeMap<String, String>,
    recipients: &EncryptRecipients,
) -> Result<()> {
    let plaintext = Zeroizing::new(toml::to_string(&SecretPayload {
        version: SECRET_PAYLOAD_VERSION,
        values: values.clone().into(),
    })?);
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
