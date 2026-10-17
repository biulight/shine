//! Compilation for workspace environments.
use super::Digest;
use super::{
    BTreeSet, Binding, CACHE_FORMAT_VERSION, Config, Context, Encryption, Path, PathBuf, Result,
    SECRET_PAYLOAD_VERSION, SecretPayload, SecretState, SensitiveValues, Sha256, SourceFile,
    Zeroize, Zeroizing, bail, effective_bindings, parse_source, secret,
};

pub(super) async fn decrypt_source_payload(
    path: &Path,
    source: &SourceFile,
    config: &Config,
) -> Result<SensitiveValues> {
    if source.payload.data.trim().is_empty() {
        return Ok(SensitiveValues::default());
    }
    let plaintext = Zeroizing::new(
        secret::decrypt_with_config(&source.payload.data, config)
            .await
            .with_context(|| format!("decrypting {}", path.display()))?,
    );
    let mut payload: SecretPayload = toml::from_str(&plaintext)
        .map_err(|_| anyhow::anyhow!("invalid decrypted environment payload"))?;
    if payload.version != SECRET_PAYLOAD_VERSION {
        bail!("unsupported encrypted payload version {}", payload.version);
    }
    Ok(std::mem::take(&mut payload.values))
}

pub(super) async fn compile_export_sources(
    sources: &[PathBuf],
    config: &Config,
    include_secrets: bool,
) -> Result<SensitiveValues> {
    let mut captured = Vec::new();
    for path in sources {
        if !path.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        captured.push((path.clone(), Some(Zeroizing::new(contents))));
    }
    compile_source_layers(&captured, config, include_secrets).await
}

pub(super) async fn compile_source_layers(
    sources: &CapturedSources,
    config: &Config,
    include_secrets: bool,
) -> Result<SensitiveValues> {
    let mut parsed = Vec::new();
    for (path, contents) in sources {
        if let Some(contents) = contents {
            parsed.push((path, parse_source(path, contents)?));
        }
    }
    if parsed.is_empty() {
        bail!("none of the configured environment source files exist");
    }
    let bindings = effective_bindings(parsed.iter().map(|(_, source)| source));
    let mut values = SensitiveValues::default();
    for (index, (path, source)) in parsed.into_iter().enumerate() {
        // Preserve validation/decryption of every source, including shadowed sources.
        if include_secrets {
            for (key, state) in &source.secret {
                if !matches!(state, SecretState::Sealed(true)) {
                    bail!(
                        "{key} in {} is not sealed; run `shine env secret seal`",
                        path.display()
                    );
                }
            }
            let secrets = decrypt_source_payload(path, &source, config).await?;
            if source.secret.keys().collect::<BTreeSet<_>>()
                != secrets.keys().collect::<BTreeSet<_>>()
            {
                bail!(
                    "secret key list does not match encrypted payload in {}",
                    path.display()
                );
            }
            for (key, mut value) in secrets {
                if bindings.get(&key) == Some(&Binding::Secret(index)) {
                    values.insert(key, value);
                } else {
                    value.zeroize();
                }
            }
        }
        for (key, mut value) in source.plain {
            if bindings.get(&key) == Some(&Binding::Plain(index)) {
                values.insert(key, value);
            } else {
                value.zeroize();
            }
        }
    }
    Ok(values)
}

pub(super) type CapturedSources = Vec<(PathBuf, Option<Zeroizing<String>>)>;

pub(super) async fn capture_sources(sources: &[PathBuf]) -> Result<CapturedSources> {
    let mut captured = Vec::new();
    for path in sources {
        let contents = match tokio::fs::read_to_string(path).await {
            Ok(contents) => Some(Zeroizing::new(contents)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("capturing environment source"),
        };
        captured.push((path.clone(), contents));
    }
    Ok(captured)
}

pub(super) fn involves_hybrid(policy: &Encryption, sources: &CapturedSources) -> Result<bool> {
    let mut hybrid = policy
        .backend
        .as_deref()
        .is_some_and(|b| b.trim().eq_ignore_ascii_case("hybrid"));
    for (path, contents) in sources {
        if let Some(contents) = contents {
            hybrid |= parse_source(path, contents)?
                .payload
                .data
                .starts_with(secret::hybrid::PREFIX);
        }
    }
    Ok(hybrid)
}

pub(super) async fn compile_captured_sources(
    sources: &CapturedSources,
    config: &Config,
) -> Result<SensitiveValues> {
    compile_source_layers(sources, config, true).await
}

pub(super) fn snapshot_input_hash(
    workspace_path: &Path,
    workspace_bytes: &str,
    mode: &str,
    sources: &CapturedSources,
) -> String {
    let mut hash = Sha256::new();
    hash.update(CACHE_FORMAT_VERSION.to_le_bytes());
    hash.update(mode.as_bytes());
    hash.update(workspace_bytes.as_bytes());
    for (path, contents) in sources {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update(
            contents
                .as_ref()
                .map_or("<missing>", |text| text.as_str())
                .as_bytes(),
        );
    }
    hash.update(workspace_path.to_string_lossy().as_bytes());
    format!("sha256:{:x}", hash.finalize())
}
