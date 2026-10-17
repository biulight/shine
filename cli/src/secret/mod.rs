//! Secret storage backends for `shine env secret encrypt`/`decrypt`.
//!
//! Two external backends exist: GPG (the original, still the default) and age, added
//! for multi-recipient encryption with Apple Touch ID support via
//! `age-plugin-se` Secure Enclave identities. Ciphertext carries a backend
//! tag (`age:<base64>`); untagged base64 continues to route to GPG so
//! secrets encrypted before age existed keep decrypting unmodified. The versioned
//! `hybrid:` envelope wraps one data key with both tools (ADR 0084).
//!
//! Encryption always needs a resolved recipient list ([`EncryptRecipients`]);
//! decryption is purely tag-based and never consults `secret_backend`, so
//! changing the default encrypt backend can never break existing secrets.

mod age;
mod exec;
mod gpg;
pub(crate) mod hybrid;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::str::FromStr;

const AGE_TAG_PREFIX: &str = "age:";

/// Which external tool a piece of ciphertext (or an encrypt request) belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackendKind {
    #[default]
    Gpg,
    Age,
    Hybrid,
}

impl FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gpg" => Ok(Self::Gpg),
            "age" => Ok(Self::Age),
            "hybrid" => Ok(Self::Hybrid),
            other => {
                bail!("unknown secret backend \"{other}\"; expected \"gpg\", \"age\" or \"hybrid\"")
            }
        }
    }
}

/// A resolved recipient list for encryption, tagged by backend.
#[derive(Clone, Debug)]
pub enum EncryptRecipients {
    Gpg(Vec<String>),
    Age(Vec<String>),
    Hybrid(hybrid::Recipients),
}

impl EncryptRecipients {
    pub fn backend(&self) -> BackendKind {
        match self {
            Self::Gpg(_) => BackendKind::Gpg,
            Self::Age(_) => BackendKind::Age,
            Self::Hybrid(_) => BackendKind::Hybrid,
        }
    }
}

/// Split stored ciphertext into its backend and undecorated payload.
/// Untagged ciphertext is treated as GPG for backward compatibility with
/// secrets encrypted before the age backend existed.
pub fn parse_tagged_ciphertext(ciphertext: &str) -> (BackendKind, &str) {
    if let Some(rest) = ciphertext.strip_prefix(hybrid::PREFIX) {
        return (BackendKind::Hybrid, rest);
    }
    match ciphertext.strip_prefix(AGE_TAG_PREFIX) {
        Some(rest) => (BackendKind::Age, rest),
        None => (BackendKind::Gpg, ciphertext),
    }
}

/// Encrypt `plaintext` for the given recipients, returning storage-ready
/// ciphertext (tagged for age, untagged for GPG).
pub async fn encrypt_secret(plaintext: &[u8], recipients: &EncryptRecipients) -> Result<String> {
    match recipients {
        EncryptRecipients::Hybrid(recipients) => hybrid::encrypt(plaintext, recipients).await,
        EncryptRecipients::Gpg(recipients) => {
            gpg::encrypt_gpg_secret_to_base64(plaintext, recipients).await
        }
        EncryptRecipients::Age(recipients) => {
            let encoded = age::encrypt_age_secret_to_base64(plaintext, recipients).await?;
            Ok(format!("{AGE_TAG_PREFIX}{encoded}"))
        }
    }
}

/// Binary file API. Unlike environment values, empty and non-UTF-8 inputs are valid.
pub async fn encrypt_bytes(plaintext: &[u8], recipients: &EncryptRecipients) -> Result<String> {
    match recipients {
        EncryptRecipients::Gpg(recipients) => gpg::encrypt_bytes(plaintext, recipients).await,
        EncryptRecipients::Age(recipients) => Ok(format!(
            "age:{}",
            age::encrypt_bytes(plaintext, recipients).await?
        )),
        EncryptRecipients::Hybrid(recipients) => hybrid::encrypt(plaintext, recipients).await,
    }
}

pub async fn decrypt_bytes(
    ciphertext: &str,
    config: &crate::config::Config,
    limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let (backend, payload) = parse_tagged_ciphertext(ciphertext);
    let identities = config.resolved_age_identities();
    let bytes = match backend {
        BackendKind::Gpg => gpg::decrypt_bytes(payload, limit).await?,
        BackendKind::Age => age::decrypt_bytes(payload, &identities, limit).await?,
        BackendKind::Hybrid => {
            hybrid::decrypt_bytes(
                payload,
                &identities,
                config.hybrid_decrypt_backend.as_deref(),
            )
            .await?
        }
    };
    anyhow::ensure!(bytes.len() <= limit, "secret output exceeds size limit");
    Ok(bytes)
}

/// Structural validation only; does not assert author authenticity or decrypt.
pub(crate) fn validate_file_ciphertext(ciphertext: &str) -> Result<()> {
    use base64::Engine;
    let (backend, payload) = parse_tagged_ciphertext(ciphertext);
    if backend == BackendKind::Hybrid {
        return hybrid::validate(payload);
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .context("invalid file ciphertext encoding")?;
    anyhow::ensure!(!decoded.is_empty(), "empty file ciphertext");
    if backend == BackendKind::Age {
        anyhow::ensure!(
            decoded.starts_with(b"age-encryption.org/v1\n"),
            "invalid age file ciphertext"
        );
    } else {
        validate_gpg_file(&decoded)?;
    }
    Ok(())
}

fn validate_gpg_file(bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        bytes.len() >= 3 && bytes[0] & 0x80 != 0,
        "invalid GPG file ciphertext"
    );
    let new = bytes[0] & 0x40 != 0;
    let tag = if new {
        bytes[0] & 0x3f
    } else {
        (bytes[0] >> 2) & 0x0f
    };
    anyhow::ensure!(
        matches!(tag, 1 | 3),
        "GPG file must begin with an encrypted session-key packet"
    );
    let (offset, length) = if new {
        match bytes[1] {
            n @ 0..=191 => (2, n as usize),
            n @ 192..=223 => (3, ((n as usize - 192) << 8) + bytes[2] as usize + 192),
            255 if bytes.len() >= 6 => (6, u32::from_be_bytes(bytes[2..6].try_into()?) as usize),
            _ => bail!("invalid GPG session-key packet length"),
        }
    } else {
        let count = match bytes[0] & 3 {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => bail!("invalid GPG session-key packet length"),
        };
        anyhow::ensure!(bytes.len() > 1 + count, "truncated GPG session-key packet");
        let length = bytes[1..1 + count]
            .iter()
            .fold(0usize, |length, byte| (length << 8) | *byte as usize);
        (1 + count, length)
    };
    anyhow::ensure!(
        length >= 10 && length < bytes.len().saturating_sub(offset),
        "truncated GPG file ciphertext"
    );
    anyhow::ensure!(
        matches!(bytes[offset], 3..=6) && bytes[offset + length] & 0x80 != 0,
        "invalid GPG file packets"
    );
    Ok(())
}

/// Check the age client before starting tagged phone pairing.
pub(crate) async fn preflight_age() -> Result<()> {
    age::preflight_age().await
}

/// Convert a legacy Secure Enclave recipient without changing its public key.
pub(crate) fn secure_enclave_recipient_to_tag(recipient: &str) -> Result<String> {
    age::secure_enclave_recipient_to_tag(recipient)
}

/// Validate the age executable and the complete recipient-side plugin set
/// before a workspace seal can decrypt any existing payload.
pub(crate) async fn preflight_age_recipients(recipients: &[String]) -> Result<()> {
    age::preflight_recipients(recipients).await
}

/// Hybrid-derived caches retain exact workspace recipient restrictions even
/// though the local cache only uses one encryption backend.
pub(crate) async fn encrypt_local_cache(
    plaintext: &[u8],
    recipients: &EncryptRecipients,
) -> Result<String> {
    match recipients {
        EncryptRecipients::Gpg(recipients) => {
            // Apply the same full-fingerprint validation as hybrid sealing.
            if recipients.is_empty()
                || recipients.iter().any(|value| {
                    value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                bail!("hybrid cache GPG recipients must be full 40-hex primary fingerprints");
            }
            let resolved = gpg::resolve_hybrid_recipients(recipients).await?;
            let encrypted = gpg::encrypt_hybrid_key(plaintext, &resolved).await?;
            Ok(exec::encode_base64_single_line(&encrypted))
        }
        EncryptRecipients::Age(_) => encrypt_secret(plaintext, recipients).await,
        EncryptRecipients::Hybrid(_) => bail!("local cache requires one backend"),
    }
}

/// Read a local hybrid-derived cache without honoring GPG output-file options.
pub(crate) async fn decrypt_local_cache(
    ciphertext: &str,
    config: &crate::config::Config,
) -> Result<String> {
    match parse_tagged_ciphertext(ciphertext) {
        (BackendKind::Gpg, payload) => gpg::decrypt_cache(payload).await,
        (BackendKind::Age, _) => decrypt_with_config(ciphertext, config).await,
        (BackendKind::Hybrid, _) => bail!("local cache requires one backend"),
    }
}

pub async fn decrypt_with_config(
    ciphertext: &str,
    config: &crate::config::Config,
) -> Result<String> {
    decrypt_with_preference(
        ciphertext,
        &config.resolved_age_identities(),
        config.hybrid_decrypt_backend.as_deref(),
    )
    .await
}

async fn decrypt_with_preference(
    ciphertext: &str,
    age_identities: &[PathBuf],
    preference: Option<&str>,
) -> Result<String> {
    let (backend, payload) = parse_tagged_ciphertext(ciphertext);
    match backend {
        BackendKind::Hybrid => hybrid::decrypt(payload, age_identities, preference).await,
        BackendKind::Gpg => gpg::decrypt_base64_gpg_secret(payload).await,
        BackendKind::Age => age::decrypt_base64_age_secret(payload, age_identities).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untagged_ciphertext_routes_to_gpg() {
        let (backend, payload) = parse_tagged_ciphertext("aGVsbG8=");
        assert_eq!(backend, BackendKind::Gpg);
        assert_eq!(payload, "aGVsbG8=");
    }

    #[test]
    fn age_tagged_ciphertext_routes_to_age_and_strips_tag() {
        let (backend, payload) = parse_tagged_ciphertext("age:aGVsbG8=");
        assert_eq!(backend, BackendKind::Age);
        assert_eq!(payload, "aGVsbG8=");
    }

    #[test]
    fn backend_kind_parses_case_insensitively() {
        assert_eq!("GPG".parse::<BackendKind>().unwrap(), BackendKind::Gpg);
        assert_eq!("Age".parse::<BackendKind>().unwrap(), BackendKind::Age);
    }

    #[test]
    fn backend_kind_rejects_unknown_values() {
        let err = "sops".parse::<BackendKind>().unwrap_err();
        assert!(
            err.to_string().contains("unknown secret backend"),
            "{err:#}"
        );
    }

    #[test]
    fn backend_kind_defaults_to_gpg() {
        assert_eq!(BackendKind::default(), BackendKind::Gpg);
    }

    #[test]
    fn encrypt_recipients_report_their_backend() {
        assert_eq!(
            EncryptRecipients::Gpg(vec!["a@example.com".to_string()]).backend(),
            BackendKind::Gpg
        );
        assert_eq!(
            EncryptRecipients::Age(vec!["age1qexample".to_string()]).backend(),
            BackendKind::Age
        );
    }
}

// Run the real backend adapters against controlled stand-ins with no base64 on PATH.
#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct RestorePath(Option<std::ffi::OsString>);

    impl Drop for RestorePath {
        fn drop(&mut self) {
            // SAFETY: the environment lock is held until after this guard drops.
            unsafe {
                match &self.0 {
                    Some(path) => std::env::set_var("PATH", path),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    #[test]
    fn both_backends_work_without_external_base64() {
        let _guard = crate::test_support::env_lock();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = runtime.block_on(crate::test_support::make_temp_dir("shine-secret-process"));
        let _restore = RestorePath(std::env::var_os("PATH"));
        for tool in ["gpg", "age"] {
            let path = dir.join(tool);
            std::fs::write(
                &path,
                r#"#!/bin/sh
case "$1" in
    --version) if test "${0##*/}" = age; then echo v1.3.0; else echo 'gpg 2.4.0'; fi ;;
    --encrypt|-e) /bin/cat ;;
    --decrypt|-d) printf '%s\n' "$@" > "$0.decrypt-called"; for arg in "$@"; do file="$arg"; done; /bin/cat "$file" ;;
    *) exit 1 ;;
esac
"#,
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let identity = dir.join("additional identity.txt");
        std::fs::write(&identity, "test identity").unwrap();
        let legacy_identity = dir.join("legacy identity.txt");
        std::fs::write(&legacy_identity, "legacy test identity").unwrap();
        // SAFETY: all environment-mutation tests hold env_lock().
        unsafe { std::env::set_var("PATH", &dir) };
        let result: Result<()> = runtime.block_on(async {
            assert!(crate::proc::ensure_command("base64").is_err());
            for recipients in [
                EncryptRecipients::Gpg(vec!["test@example.com".into()]),
                EncryptRecipients::Age(vec!["age1tag1test".into()]),
            ] {
                let plaintext = "secret\nwith trailing newline\n";
                let encoded = encrypt_secret(plaintext.as_bytes(), &recipients).await?;
                let expected = "c2VjcmV0CndpdGggdHJhaWxpbmcgbmV3bGluZQo=";
                assert_eq!(
                    encoded,
                    match recipients {
                        EncryptRecipients::Gpg(_) => expected.to_string(),
                        EncryptRecipients::Age(_) => format!("age:{expected}"),
                        EncryptRecipients::Hybrid(_) => unreachable!(),
                    }
                );
                let mut config = crate::config::Config::new_for_test(&dir);
                config.age_identity = Some(legacy_identity.to_str().unwrap().to_string());
                config.age_identities = vec![
                    identity.to_str().unwrap().to_string(),
                    legacy_identity.to_str().unwrap().to_string(),
                ];
                // The encryption default must not override the stored ciphertext tag.
                config.secret_backend = Some(
                    match recipients {
                        EncryptRecipients::Gpg(_) => "age",
                        EncryptRecipients::Age(_) => "gpg",
                        EncryptRecipients::Hybrid(_) => unreachable!(),
                    }
                    .into(),
                );
                assert_eq!(decrypt_with_config(&encoded, &config).await?, plaintext);
                for tool in ["gpg", "age"] {
                    let called = dir.join(format!("{tool}.decrypt-called"));
                    assert_eq!(
                        called.is_file(),
                        tool.parse::<BackendKind>().unwrap() == recipients.backend(),
                        "ciphertext tag must select exactly its backend"
                    );
                    if called.is_file() {
                        if tool == "age" {
                            let arguments = std::fs::read_to_string(&called)?;
                            let arguments = arguments.lines().collect::<Vec<_>>();
                            assert_eq!(arguments.len(), 6);
                            assert_eq!(
                                &arguments[..5],
                                [
                                    "-d",
                                    "-i",
                                    legacy_identity.to_str().unwrap(),
                                    "-i",
                                    identity.to_str().unwrap(),
                                ],
                                "configured legacy and additional identities must reach age once each"
                            );
                        }
                        std::fs::remove_file(called)?;
                    }
                }
            }

            let legacy = EncryptRecipients::Age(vec!["age1se1legacy".into()]);
            let missing = encrypt_secret(b"secret", &legacy).await.unwrap_err();
            assert!(
                missing
                    .to_string()
                    .contains("shine state migrate --dry-run")
            );
            std::fs::write(dir.join("age-plugin-se"), "").unwrap();
            assert!(encrypt_secret(b"secret", &legacy).await.is_ok());
            Ok(())
        });
        std::fs::remove_dir_all(&dir).unwrap();
        result.unwrap();
    }

    #[test]
    fn age_version_is_enforced_at_the_process_boundary() {
        let _guard = crate::test_support::env_lock();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = runtime.block_on(crate::test_support::make_temp_dir("shine-age-version"));
        let age = dir.join("age");
        let _restore = RestorePath(std::env::var_os("PATH"));
        // SAFETY: all environment-mutation tests hold env_lock().
        unsafe { std::env::set_var("PATH", &dir) };

        for (output, accepted) in [("v1.2.0", false), ("unexpected", false), ("v1.3.0", true)] {
            std::fs::write(&age, format!("#!/bin/sh\necho '{output}'\n")).unwrap();
            std::fs::set_permissions(&age, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(runtime.block_on(age::preflight_age()).is_ok(), accepted);
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
