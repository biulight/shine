//! Validation and decryption of one captured source, independent of release policy.
//! Sealing can read pending sources; compilation and broker release require sealed declarations.
use super::{
    BTreeSet, Config, Context, Path, Result, SECRET_PAYLOAD_VERSION, SecretPayload, SecretState,
    SensitiveValues, SourceFile, Zeroizing, bail, secret,
};

#[derive(Clone, Copy)]
pub(super) enum SealHint {
    Include,
    Omit,
}

pub(super) fn validate_sealed_source(
    path: &Path,
    source: &SourceFile,
    hint: SealHint,
) -> Result<()> {
    for (key, state) in &source.secret {
        if !matches!(state, SecretState::Sealed(true)) {
            let suffix = match hint {
                SealHint::Include => "; run `shine env secret seal`",
                SealHint::Omit => "",
            };
            bail!("{key} in {} is not sealed{suffix}", path.display());
        }
    }
    Ok(())
}

pub(super) async fn decrypt_sealed_source(
    path: &Path,
    source: &SourceFile,
    config: &Config,
    hint: SealHint,
) -> Result<SensitiveValues> {
    validate_sealed_source(path, source, hint)?;
    let secrets = decrypt_source_payload(path, source, config).await?;
    if source.secret.keys().collect::<BTreeSet<_>>() != secrets.keys().collect::<BTreeSet<_>>() {
        bail!(
            "secret key list does not match encrypted payload in {}",
            path.display()
        );
    }
    Ok(secrets)
}

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
