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
    let project_id = shine_core::digest::sha256_hex(canonical.as_os_str().as_encoded_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(unix, windows))]
    #[test]
    fn cache_project_keys_distinguish_unrepresentable_native_paths() {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(b"project-\xff".to_vec())
        };
        #[cfg(windows)]
        let raw = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[0xd800])
        };
        let root = std::env::temp_dir().join(format!("shine-cache-{}", uuid::Uuid::new_v4()));
        let native = root.join(&raw).join("shine.workspace.toml");
        let alias = root
            .join(raw.to_string_lossy().as_ref())
            .join("shine.workspace.toml");
        assert_ne!(native, alias);
        for mode in ["development", "production"] {
            assert_ne!(
                cache_path(&native, mode).unwrap(),
                cache_path(&alias, mode).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn cache_project_keys_keep_utf8_and_canonical_directory_identity() {
        let root = crate::test_support::make_temp_dir("shine-cache-path").await;
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let canonical = std::fs::canonicalize(&root).unwrap();
        let expected_id = shine_core::digest::sha256_hex(canonical.to_str().unwrap().as_bytes());
        let direct = cache_path(&root.join("shine.workspace.toml"), "development").unwrap();
        assert_eq!(direct.file_name().unwrap(), "env-development.toml");
        assert_eq!(
            direct.parent().unwrap().file_name().unwrap(),
            expected_id.as_str()
        );
        assert_eq!(
            direct,
            cache_path(
                &nested.join("..").join("shine.workspace.toml"),
                "development"
            )
            .unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let native = root.join(std::ffi::OsString::from_vec(b"project-\xff".to_vec()));
            if std::fs::create_dir(&native).is_ok() {
                let alias = root.join("project-\u{fffd}");
                std::fs::create_dir(&alias).unwrap();
                assert_ne!(
                    cache_path(&native.join("shine.workspace.toml"), "development").unwrap(),
                    cache_path(&alias.join("shine.workspace.toml"), "development").unwrap()
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
