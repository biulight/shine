//! GPG-backed secret storage: base64-encoded ciphertext round-tripped through
//! the `gpg` CLI with in-process Base64 encoding. Ciphertext carries no backend
//! tag, so `secret::mod` treats untagged base64 as GPG for backward
//! compatibility with secrets encrypted before other backends existed.

use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::process::Command;

use super::exec::{
    TempFile, decode_base64_to_file, encode_base64_single_line, write_stdin_and_wait,
};
use crate::proc::ensure_command;

pub async fn decrypt_base64_gpg_secret(encoded_secret: &str) -> Result<String> {
    decrypt_base64_gpg(encoded_secret, false, false).await
}

pub(super) async fn decrypt_hybrid_key(encoded: &str) -> Result<String> {
    decrypt_base64_gpg(encoded, true, true).await
}

pub(super) async fn decrypt_cache(encoded: &str) -> Result<String> {
    decrypt_base64_gpg(encoded, false, true).await
}

async fn decrypt_base64_gpg(encoded_secret: &str, key: bool, strict: bool) -> Result<String> {
    if encoded_secret.trim().is_empty() {
        bail!("secret is empty");
    }

    ensure_command("gpg")?;

    let encrypted_file = TempFile::new("shine-gpg-secret").await?;
    decode_base64_to_file(encoded_secret, encrypted_file.path()).await?;
    let encrypted_meta = tokio::fs::metadata(encrypted_file.path())
        .await
        .with_context(|| format!("reading {}", encrypted_file.path().display()))?;
    if encrypted_meta.len() == 0 {
        bail!("decoded secret is empty");
    }

    decrypt_gpg_file(encrypted_file.path(), key, strict).await
}

pub async fn encrypt_gpg_secret_to_base64(
    plaintext: &[u8],
    recipients: &[String],
) -> Result<String> {
    if plaintext.is_empty() {
        bail!("secret is empty");
    }
    let recipients = validate_recipients(recipients)?;

    ensure_command("gpg")?;

    let encrypted = encrypt_gpg(plaintext, &recipients).await?;
    Ok(encode_base64_single_line(&encrypted))
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

async fn decrypt_gpg_file(path: &Path, key: bool, strict: bool) -> Result<String> {
    let mut command = Command::new("gpg");
    if strict {
        command.args(["--no-options", "--output", "-"]);
    }
    let output = command
        .kill_on_drop(true)
        .arg("--decrypt")
        .arg(path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(if key {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::inherit()
        })
        .spawn()
        .with_context(|| "running gpg --decrypt")?;

    if key {
        return super::exec::read_key_output(output).await;
    }
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

// Hybrid alone ignores local option files and freezes exact encryption-key identities.
fn hybrid_command() -> Command {
    let mut command = Command::new("gpg");
    command.args([
        "--no-options",
        "--batch",
        "--no-auto-key-locate",
        "--no-auto-key-retrieve",
        "--no-auto-check-trustdb",
        "--no-encrypt-to",
        "--no-default-recipient",
    ]);
    command
}

pub(super) async fn resolve_hybrid_recipients(recipients: &[String]) -> Result<Vec<String>> {
    let mut resolved = Vec::new();
    for fingerprint in recipients {
        let output = hybrid_command()
            .args([
                "--with-colons",
                "--fixed-list-mode",
                "--with-fingerprint",
                "--with-subkey-fingerprint",
                "--list-keys",
                fingerprint,
            ])
            .output()
            .await
            .context("resolving hybrid GPG public fingerprint")?;
        if !output.status.success() {
            bail!("hybrid GPG public key unavailable");
        }
        resolved.push(select_encryption_key(
            &String::from_utf8(output.stdout).context("invalid GPG public metadata")?,
            fingerprint,
        )?);
    }
    Ok(resolved)
}

fn select_encryption_key(metadata: &str, expected: &str) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mut primary_count = 0;
    let mut primary_valid = false;
    let mut primary_matched = false;
    let mut pending = None;
    let mut encryption_key = None;
    for line in metadata.lines() {
        let fields: Vec<_> = line.split(':').collect();
        let field = |n| fields.get(n).copied().unwrap_or("");
        match field(0) {
            "pub" | "sub" => {
                let primary = field(0) == "pub";
                let valid = !matches!(field(1), "r" | "e" | "d" | "i")
                    && !field(11).contains('D')
                    && (field(6).is_empty()
                        || field(6) == "0"
                        || field(6).parse::<u64>().is_ok_and(|expiry| expiry > now));
                if primary {
                    primary_count += 1;
                    primary_valid = valid;
                }
                pending = Some((primary, valid && field(11).contains('e')));
            }
            "fpr" => {
                if let Some((primary, encrypts)) = pending.take() {
                    let fingerprint = field(9);
                    if fingerprint.len() != 40
                        || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        bail!("invalid hybrid GPG fingerprint metadata");
                    }
                    if primary {
                        primary_matched = fingerprint.eq_ignore_ascii_case(expected);
                    }
                    if encrypts {
                        encryption_key = Some(format!("{fingerprint}!"));
                    }
                }
            }
            _ => {}
        }
    }
    if primary_count != 1 || !primary_valid || !primary_matched {
        bail!("hybrid GPG recipient must resolve to one valid matching primary public key");
    }
    encryption_key.context("hybrid GPG recipient has no valid encryption key")
}

pub(super) async fn encrypt_hybrid_key(plaintext: &[u8], recipients: &[String]) -> Result<Vec<u8>> {
    let mut command = hybrid_command();
    command.args(["--trust-model", "always", "--encrypt", "--output", "-"]);
    for recipient in recipients {
        command.arg("--recipient").arg(recipient);
    }
    let child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("running hybrid GPG encryption")?;
    let output = write_stdin_and_wait(child, plaintext).await?;
    if !output.status.success() {
        bail!("hybrid GPG encryption failed; current source was not updated");
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_public_resolution_rejects_invalid_or_ambiguous_primary_keys() {
        let primary = "A".repeat(40);
        let subkey = "B".repeat(40);
        let metadata = format!(
            "pub:u:2048:1:0:0:0:::::sc:\nfpr:::::::::{primary}:\nsub:u:2048:1:0:0:0:::::e:\nfpr:::::::::{subkey}:\n"
        );
        assert_eq!(
            select_encryption_key(&metadata, &primary).unwrap(),
            format!("{subkey}!")
        );
        assert!(select_encryption_key(&metadata, &subkey).is_err());
        for validity in ["r", "e", "d", "i"] {
            assert!(
                select_encryption_key(
                    &metadata.replacen("pub:u:", &format!("pub:{validity}:"), 1),
                    &primary
                )
                .is_err()
            );
        }
        for invalid in [
            metadata.replace(":sc:", ":scD:"),
            metadata.replace("pub:u:2048:1:0:0:0:", "pub:u:2048:1:0:0:1:"),
            metadata.replace("sub:u:", "sub:r:"),
            format!("{metadata}{metadata}"),
        ] {
            assert!(select_encryption_key(&invalid, &primary).is_err());
        }
    }

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
