//! age-backed secret storage: base64-encoded ciphertext round-tripped through
//! the `age` CLI, supporting multi-recipient encryption so a secret sealed
//! once can be decrypted by any teammate's identity — including Secure
//! Enclave identities minted by `age-plugin-se`, which prompt Touch ID on
//! decrypt. Ciphertext is tagged `age:` by the router in `secret::mod` so it
//! is never confused with untagged GPG ciphertext.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

use super::exec::{
    TempFile, decode_base64_to_file, encode_base64_single_line, write_stdin_and_wait,
};
use crate::proc::ensure_command;

pub async fn encrypt_age_secret_to_base64(
    plaintext: &[u8],
    recipients: &[String],
) -> Result<String> {
    if plaintext.is_empty() {
        bail!("secret is empty");
    }
    let recipients = validate_recipients(recipients)?;

    ensure_command("base64")?;
    ensure_command("age")?;

    let encrypted = encrypt_age(plaintext, &recipients).await?;
    encode_base64_single_line(&encrypted).await
}

pub async fn decrypt_base64_age_secret(
    encoded_secret: &str,
    identities: &[PathBuf],
) -> Result<String> {
    if encoded_secret.trim().is_empty() {
        bail!("secret is empty");
    }
    if identities.is_empty() {
        bail!(
            "no age identity configured; run `shine env identity init` or set age_identity in config.toml"
        );
    }
    let needs_plugin = any_identity_uses_secure_enclave(identities).await?;

    ensure_command("base64")?;
    ensure_command("age")?;
    if needs_plugin {
        ensure_command("age-plugin-se")?;
    }

    let encrypted_file = TempFile::new("shine-age-secret").await?;
    decode_base64_to_file(encoded_secret, encrypted_file.path()).await?;
    let encrypted_meta = tokio::fs::metadata(encrypted_file.path())
        .await
        .with_context(|| format!("reading {}", encrypted_file.path().display()))?;
    if encrypted_meta.len() == 0 {
        bail!("decoded secret is empty");
    }

    decrypt_age_file(encrypted_file.path(), identities).await
}

fn validate_recipients(recipients: &[String]) -> Result<Vec<&str>> {
    if recipients.is_empty() {
        bail!("recipients is empty");
    }
    let mut cleaned = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        let trimmed = recipient.trim();
        if trimmed.is_empty() {
            bail!("recipient is empty");
        }
        if trimmed.chars().any(char::is_whitespace) {
            bail!("recipient must not contain whitespace: {trimmed}");
        }
        cleaned.push(trimmed);
    }
    Ok(cleaned)
}

async fn any_identity_uses_secure_enclave(identities: &[PathBuf]) -> Result<bool> {
    for identity in identities {
        if !identity.is_file() {
            bail!("age identity file not found: {}", identity.display());
        }
        let contents = tokio::fs::read_to_string(identity)
            .await
            .with_context(|| format!("reading age identity {}", identity.display()))?;
        if contents.contains("AGE-PLUGIN-SE-") {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn decrypt_age_file(path: &Path, identities: &[PathBuf]) -> Result<String> {
    let mut command = Command::new("age");
    command.arg("-d");
    for identity in identities {
        command.arg("-i").arg(identity);
    }
    command.arg(path);

    let output = command
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| "running age -d")?;

    let output = output
        .wait_with_output()
        .await
        .context("waiting for age -d")?;
    if !output.status.success() {
        bail!("age decrypt failed");
    }

    String::from_utf8(output.stdout).context("decrypted secret is not valid UTF-8")
}

async fn encrypt_age(plaintext: &[u8], recipients: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("age");
    command.arg("-e");
    for recipient in recipients {
        command.arg("-r").arg(recipient);
    }

    let output = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| "running age -e")?;

    let output = write_stdin_and_wait(output, plaintext).await?;
    if !output.status.success() {
        bail!("age encrypt failed");
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_secret_fails_before_external_commands() {
        let err = decrypt_base64_age_secret("", &[PathBuf::from("/does/not/matter")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("secret is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn empty_identities_fails_before_external_commands() {
        let err = decrypt_base64_age_secret("c29tZS1jaXBoZXJ0ZXh0", &[])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no age identity configured"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn empty_plaintext_fails_before_external_commands() {
        let err = encrypt_age_secret_to_base64(b"", &["age1qexample".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("secret is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn empty_recipients_fails_before_external_commands() {
        let err = encrypt_age_secret_to_base64(b"secret", &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("recipients is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn blank_recipient_fails_before_external_commands() {
        let err = encrypt_age_secret_to_base64(b"secret", &["  ".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("recipient is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn whitespace_in_recipient_fails_before_external_commands() {
        let err = encrypt_age_secret_to_base64(b"secret", &["age1 abc".to_string()])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("must not contain whitespace"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn missing_identity_file_fails_before_decrypt() {
        let missing = PathBuf::from("/tmp/shine-age-identity-does-not-exist-in-test");
        let err = decrypt_base64_age_secret("c29tZS1jaXBoZXJ0ZXh0", &[missing])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("age identity file not found"),
            "{err:#}"
        );
    }
}
