//! Personal workspace preferences and single-backend hybrid compilation caches.
//! Deliberately called only by local workspace handlers, never broker snapshots.
use super::*;

const LOCAL_CONFIG: &str = "shine.config.local.toml";
const VERSION: u32 = 1;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalConfig {
    hybrid_decrypt_backend: Option<String>,
}

pub(super) async fn load_config(config: &Config, workspace: &Path) -> Result<Config> {
    let path = workspace
        .parent()
        .context("workspace has no parent")?
        .join(LOCAL_CONFIG);
    let mut effective = config.clone();
    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(effective),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let local: LocalConfig =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    if let Some(preference) = local.hybrid_decrypt_backend {
        let backend: BackendKind = preference.parse()?;
        if backend == BackendKind::Hybrid {
            bail!(
                "{}: hybrid_decrypt_backend must be gpg or age",
                path.display()
            );
        }
        effective.hybrid_decrypt_backend = Some(backend_name(backend).into());
    }
    Ok(effective)
}

fn backend_name(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Gpg => "gpg",
        BackendKind::Age => "age",
        BackendKind::Hybrid => unreachable!("local cache requires a selected backend"),
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u32,
    context: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    context: String,
    payload: SecretPayload,
}

fn cache_context(input_hash: &str, backend: BackendKind, recipients: &[String]) -> String {
    // Length-delimited serialization binds boundaries as well as values.
    let encoded = serde_json::to_vec(&(VERSION, input_hash, backend_name(backend), recipients))
        .expect("cache context is serializable");
    format!("sha256:{:x}", Sha256::digest(encoded))
}

fn validate_sources(sources: &CapturedSources) -> Result<bool> {
    let mut has_payload = false;
    for (path, contents) in sources {
        let Some(contents) = contents else { continue };
        let source = parse_source(path, contents)?;
        has_payload |= !source.payload.data.trim().is_empty();
        for (key, state) in &source.secret {
            if !matches!(state, SecretState::Sealed(true)) {
                bail!(
                    "{key} in {} is not sealed; run `shine env secret seal`",
                    path.display()
                );
            }
        }
        if let Some(encoded) = source.payload.data.strip_prefix(secret::hybrid::PREFIX) {
            secret::hybrid::validate(encoded)?;
        }
    }
    Ok(has_payload)
}

pub(super) async fn compile(
    config: &Config,
    workspace: &Path,
    mode: &str,
    input_hash: &str,
    policy: &Encryption,
    sources: &CapturedSources,
) -> Result<BTreeMap<String, String>> {
    // Reject malformed hybrid sources before selection or any cache decryption.
    if !validate_sources(sources)? {
        return compile_captured_sources(sources, config).await;
    }
    let selected = secret::hybrid::select(
        &config.resolved_age_identities(),
        config.hybrid_decrypt_backend.as_deref(),
    )
    .await?;
    let mut effective = config.clone();
    effective.hybrid_decrypt_backend = Some(backend_name(selected).into());
    let recipients = clean_recipients(match selected {
        BackendKind::Gpg => &policy.gpg_recipients,
        BackendKind::Age => &policy.age_recipients,
        BackendKind::Hybrid => unreachable!(),
    });
    // Never widen a workspace's access list with global recipients.
    if recipients.is_empty() {
        return compile_captured_sources(sources, &effective).await;
    }
    let context = cache_context(input_hash, selected, &recipients);
    let path = cache_path(workspace, mode)?.with_file_name(format!(
        "env-{mode}-hybrid-local-{}.toml",
        backend_name(selected)
    ));
    if let Some(values) = read_cache(&path, &context, selected, &effective).await? {
        return Ok(values);
    }
    let values = compile_captured_sources(sources, &effective).await?;
    let encryption = match selected {
        BackendKind::Gpg => EncryptRecipients::Gpg(recipients),
        BackendKind::Age => EncryptRecipients::Age(recipients),
        BackendKind::Hybrid => unreachable!(),
    };
    if let Err(error) = write_local_cache(&path, &context, &values, &encryption).await {
        eprintln!("Warning: could not update environment cache: {error:#}");
    }
    Ok(values)
}

async fn read_cache(
    path: &Path,
    context: &str,
    selected: BackendKind,
    config: &Config,
) -> Result<Option<BTreeMap<String, String>>> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            eprintln!("Warning: ignoring unreadable environment cache: {error}");
            return Ok(None);
        }
    };
    let entry: Entry = match toml::from_str(&contents) {
        Ok(entry) => entry,
        Err(_) => return Ok(None),
    };
    if entry.version != VERSION
        || entry.context != context
        || secret::parse_tagged_ciphertext(&entry.data).0 != selected
    {
        return Ok(None);
    }
    // Once decryption starts, errors/cancellation are terminal: no second prompt
    // from source compilation and no automatic switch to another backend.
    let plaintext = Zeroizing::new(secret::decrypt_local_cache(&entry.data, config).await?);
    decode_payload(&plaintext, context).map(Some)
}

fn decode_payload(plaintext: &str, context: &str) -> Result<BTreeMap<String, String>> {
    let mut payload: Payload = toml::from_str(plaintext)
        .map_err(|_| anyhow::anyhow!("invalid encrypted environment cache payload"))?;
    if payload.version != VERSION
        || payload.context != context
        || payload.payload.version != SECRET_PAYLOAD_VERSION
    {
        bail!("compiled environment cache failed integrity validation");
    }
    Ok(std::mem::take(&mut payload.payload.values))
}

async fn write_local_cache(
    path: &Path,
    context: &str,
    values: &BTreeMap<String, String>,
    encryption: &EncryptRecipients,
) -> Result<()> {
    let plaintext = Zeroizing::new(toml::to_string(&Payload {
        version: VERSION,
        context: context.into(),
        payload: SecretPayload {
            version: SECRET_PAYLOAD_VERSION,
            values: values.clone(),
        },
    })?);
    let data = secret::encrypt_local_cache(plaintext.as_bytes(), encryption).await?;
    let contents = toml::to_string(&Entry {
        version: VERSION,
        context: context.into(),
        data,
    })?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    atomic_write_private(path, contents.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plaintext_needs_no_hybrid_backend_and_unsealed_sources_fail_early() {
        let dir = crate::test_support::make_temp_dir("shine-plain-hybrid").await;
        let mut config = Config::new_for_test(&dir);
        // Selection would fail; a secretless source must never reach selection.
        config.hybrid_decrypt_backend = Some("hybrid".into());
        let source = dir.join("source.toml");
        let captured = vec![(
            source.clone(),
            Some(Zeroizing::new("[plain]\nVALUE = 'plain'\n".into())),
        )];
        assert_eq!(
            compile(
                &config,
                &dir.join("shine.workspace.toml"),
                "test",
                "input",
                &Encryption::default(),
                &captured
            )
            .await
            .unwrap()["VALUE"],
            "plain"
        );
        let unsealed = vec![(
            source,
            Some(Zeroizing::new("[secret]\nTOKEN = 'pending'\n".into())),
        )];
        assert!(
            validate_sources(&unsealed)
                .unwrap_err()
                .to_string()
                .contains("not sealed")
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn local_preference_is_sparse_and_never_changes_caller_config() {
        let dir = crate::test_support::make_temp_dir("shine-local-config").await;
        let workspace = dir.join("shine.workspace.toml");
        let mut config = Config::new_for_test(&dir);
        config.hybrid_decrypt_backend = Some("gpg".into());
        assert_eq!(
            load_config(&config, &workspace)
                .await
                .unwrap()
                .hybrid_decrypt_backend
                .as_deref(),
            Some("gpg")
        );
        let path = dir.join(LOCAL_CONFIG);
        tokio::fs::write(&path, "hybrid_decrypt_backend = 'age'")
            .await
            .unwrap();
        assert_eq!(
            load_config(&config, &workspace)
                .await
                .unwrap()
                .hybrid_decrypt_backend
                .as_deref(),
            Some("age")
        );
        assert_eq!(config.hybrid_decrypt_backend.as_deref(), Some("gpg"));
        tokio::fs::write(&path, "# inherit").await.unwrap();
        assert_eq!(
            load_config(&config, &workspace)
                .await
                .unwrap()
                .hybrid_decrypt_backend
                .as_deref(),
            Some("gpg")
        );
        for invalid in [
            "hybrid_decrypt_backend = 'hybrid'",
            "hybrid_decrypt_backend = 'invalid'",
            "hybrid_decrypt_backend = [",
            "age_recipients = ['other']",
        ] {
            tokio::fs::write(&path, invalid).await.unwrap();
            assert!(load_config(&config, &workspace).await.is_err());
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn encrypted_context_rejects_relabelled_cache() {
        let recipients = vec!["age1test".into()];
        let context = cache_context("source-v1", BackendKind::Age, &recipients);
        let plaintext = toml::to_string(&Payload {
            version: VERSION,
            context: context.clone(),
            payload: SecretPayload {
                version: SECRET_PAYLOAD_VERSION,
                values: BTreeMap::from([("TOKEN".into(), "value".into())]),
            },
        })
        .unwrap();
        assert_eq!(
            decode_payload(&plaintext, &context).unwrap()["TOKEN"],
            "value"
        );
        for changed in [
            cache_context("source-v2", BackendKind::Age, &recipients),
            cache_context("source-v1", BackendKind::Gpg, &recipients),
            cache_context("source-v1", BackendKind::Age, &["age1other".into()]),
        ] {
            assert!(decode_payload(&plaintext, &changed).is_err());
        }
    }

    #[tokio::test]
    async fn wrong_backend_cache_is_rejected_before_tool_invocation() {
        let dir = crate::test_support::make_temp_dir("shine-local-cache").await;
        let path = dir.join("cache.toml");
        let config = Config::new_for_test(&dir);
        for data in ["age:not-valid", "hybrid:not-valid"] {
            tokio::fs::write(
                &path,
                toml::to_string(&Entry {
                    version: VERSION,
                    context: "matching".into(),
                    data: data.into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
            assert!(
                read_cache(&path, "matching", BackendKind::Gpg, &config)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
