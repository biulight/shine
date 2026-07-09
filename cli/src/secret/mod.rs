//! Secret storage backends for `shine env encrypt`/`decrypt`.
//!
//! Two backends exist: GPG (the original, still the default) and age, added
//! for multi-recipient encryption with Apple Touch ID support via
//! `age-plugin-se` Secure Enclave identities. Ciphertext carries a backend
//! tag (`age:<base64>`); untagged base64 continues to route to GPG so
//! secrets encrypted before age existed keep decrypting unmodified.
//!
//! Encryption always needs a resolved recipient list ([`EncryptRecipients`]);
//! decryption is purely tag-based and never consults `secret_backend`, so
//! changing the default encrypt backend can never break existing secrets.

mod age;
mod exec;
mod gpg;

pub(crate) use exec::ensure_command;

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::str::FromStr;

const AGE_TAG_PREFIX: &str = "age:";

/// Which external tool a piece of ciphertext (or an encrypt request) belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackendKind {
    #[default]
    Gpg,
    Age,
}

impl FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gpg" => Ok(Self::Gpg),
            "age" => Ok(Self::Age),
            other => bail!("unknown secret backend \"{other}\"; expected \"gpg\" or \"age\""),
        }
    }
}

/// A resolved recipient list for encryption, tagged by backend.
#[derive(Clone, Debug)]
pub enum EncryptRecipients {
    Gpg(Vec<String>),
    Age(Vec<String>),
}

impl EncryptRecipients {
    pub fn backend(&self) -> BackendKind {
        match self {
            Self::Gpg(_) => BackendKind::Gpg,
            Self::Age(_) => BackendKind::Age,
        }
    }
}

/// Split stored ciphertext into its backend and undecorated payload.
/// Untagged ciphertext is treated as GPG for backward compatibility with
/// secrets encrypted before the age backend existed.
pub fn parse_tagged_ciphertext(ciphertext: &str) -> (BackendKind, &str) {
    match ciphertext.strip_prefix(AGE_TAG_PREFIX) {
        Some(rest) => (BackendKind::Age, rest),
        None => (BackendKind::Gpg, ciphertext),
    }
}

/// Encrypt `plaintext` for the given recipients, returning storage-ready
/// ciphertext (tagged for age, untagged for GPG).
pub async fn encrypt_secret(plaintext: &[u8], recipients: &EncryptRecipients) -> Result<String> {
    match recipients {
        EncryptRecipients::Gpg(recipients) => {
            gpg::encrypt_gpg_secret_to_base64(plaintext, recipients).await
        }
        EncryptRecipients::Age(recipients) => {
            let encoded = age::encrypt_age_secret_to_base64(plaintext, recipients).await?;
            Ok(format!("{AGE_TAG_PREFIX}{encoded}"))
        }
    }
}

/// Decrypt stored ciphertext, routing purely on its tag. `age_identities` is
/// only consulted when the ciphertext is tagged `age:`.
pub async fn decrypt_secret(ciphertext: &str, age_identities: &[PathBuf]) -> Result<String> {
    let (backend, payload) = parse_tagged_ciphertext(ciphertext);
    match backend {
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
