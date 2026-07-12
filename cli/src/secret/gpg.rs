//! GPG-backed secret storage: base64-encoded ciphertext round-tripped through
//! the `gpg` and `base64` CLI tools. Ciphertext carries no backend tag, so the
//! router in `secret::mod` treats untagged base64 as GPG for backward
//! compatibility with secrets encrypted before other backends existed.

use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::process::Command;

use super::exec::{
    TempFile, decode_base64_to_file, encode_base64_single_line, write_stdin_and_wait,
};
use crate::proc::ensure_command;

pub async fn decrypt_base64_gpg_secret(encoded_secret: &str) -> Result<String> {
    if encoded_secret.trim().is_empty() {
        bail!("secret is empty");
    }

    ensure_command("base64")?;
    ensure_command("gpg")?;

    let encrypted_file = TempFile::new("shine-gpg-secret").await?;
    decode_base64_to_file(encoded_secret, encrypted_file.path()).await?;
    let encrypted_meta = tokio::fs::metadata(encrypted_file.path())
        .await
        .with_context(|| format!("reading {}", encrypted_file.path().display()))?;
    if encrypted_meta.len() == 0 {
        bail!("decoded secret is empty");
    }

    decrypt_gpg_file(encrypted_file.path()).await
}

pub async fn encrypt_gpg_secret_to_base64(
    plaintext: &[u8],
    recipients: &[String],
) -> Result<String> {
    if plaintext.is_empty() {
        bail!("secret is empty");
    }
    let recipients = validate_recipients(recipients)?;

    ensure_command("base64")?;
    ensure_command("gpg")?;

    let encrypted = encrypt_gpg(plaintext, &recipients).await?;
    encode_base64_single_line(&encrypted).await
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
        cleaned.push(trimmed);
    }
    Ok(cleaned)
}

async fn decrypt_gpg_file(path: &Path) -> Result<String> {
    let output = Command::new("gpg")
        .arg("--decrypt")
        .arg(path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| "running gpg --decrypt")?;

    let output = output
        .wait_with_output()
        .await
        .context("waiting for gpg --decrypt")?;
    if !output.status.success() {
        bail!("gpg decrypt failed");
    }

    String::from_utf8(output.stdout).context("decrypted secret is not valid UTF-8")
}

async fn encrypt_gpg(plaintext: &[u8], recipients: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("gpg");
    command.arg("--encrypt");
    for recipient in recipients {
        command.arg("-r").arg(recipient);
    }

    let output = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| "running gpg --encrypt")?;

    let output = write_stdin_and_wait(output, plaintext).await?;
    if !output.status.success() {
        bail!("gpg encrypt failed");
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_secret_fails_before_external_commands() {
        let err = decrypt_base64_gpg_secret("").await.unwrap_err();
        assert!(err.to_string().contains("secret is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn empty_plaintext_fails_before_external_commands() {
        let err = encrypt_gpg_secret_to_base64(b"", &["test@example.com".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("secret is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn empty_recipients_fails_before_external_commands() {
        let err = encrypt_gpg_secret_to_base64(b"secret", &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("recipients is empty"), "{err:#}");
    }

    #[tokio::test]
    async fn blank_recipient_fails_before_external_commands() {
        let err = encrypt_gpg_secret_to_base64(b"secret", &["  ".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("recipient is empty"), "{err:#}");
    }
}
