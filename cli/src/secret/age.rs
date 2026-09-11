//! age-backed secret storage: base64-encoded ciphertext round-tripped through
//! the `age` CLI, supporting multi-recipient encryption so a secret sealed
//! once can be decrypted by any teammate's identity — including Secure
//! Enclave and phone-backed identities, which require an independent user
//! authorization on decrypt. Ciphertext is tagged `age:` by the router in `secret::mod` so it
//! is never confused with untagged GPG ciphertext.

use anyhow::{Context, Result, bail};
use bech32::{self, Variant};
use semver::Version;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tokio::process::Command;

use super::exec::{
    TempFile, decode_base64_to_file, encode_base64_single_line, write_stdin_and_wait,
};
use crate::proc::ensure_command;

const MINIMUM_AGE_VERSION: &str = "1.3.0";

pub async fn encrypt_age_secret_to_base64(
    plaintext: &[u8],
    recipients: &[String],
) -> Result<String> {
    if plaintext.is_empty() {
        bail!("secret is empty");
    }
    let recipients = validate_recipients(recipients)?;

    preflight_age().await?;
    preflight_recipient_plugins(&recipients)?;

    let encrypted = encrypt_age(plaintext, &recipients).await?;
    Ok(encode_base64_single_line(&encrypted))
}

pub async fn decrypt_base64_age_secret(
    encoded_secret: &str,
    identities: &[PathBuf],
) -> Result<String> {
    decrypt_base64_age(encoded_secret, identities, false).await
}

pub(super) async fn decrypt_hybrid_key(encoded: &str, identities: &[PathBuf]) -> Result<String> {
    decrypt_base64_age(encoded, identities, true).await
}

async fn decrypt_base64_age(
    encoded_secret: &str,
    identities: &[PathBuf],
    key: bool,
) -> Result<String> {
    if encoded_secret.trim().is_empty() {
        bail!("secret is empty");
    }
    if identities.is_empty() {
        bail!(
            "no age identity configured; run `shine env secret identity init` or set age_identity in config.toml"
        );
    }
    let identity_plugins = inspect_identity_plugins(identities).await?;
    let quiet_phone_progress = identity_plugins.required.contains(&"age-plugin-phone")
        && !phone_terminal_output_requested(
            std::env::var_os("AGE_PLUGIN_PHONE_TRANSPORT").as_deref(),
            std::env::var_os("AGE_PLUGIN_PHONE_MESSAGES").as_deref(),
        );
    let multiple_phone_identities = identity_plugins.phone_identity_count > 1;

    preflight_age().await?;
    for plugin in identity_plugins.required {
        ensure_identity_plugin(plugin)?;
    }

    let encrypted_file = TempFile::new("shine-age-secret").await?;
    decode_base64_to_file(encoded_secret, encrypted_file.path()).await?;
    let encrypted_meta = tokio::fs::metadata(encrypted_file.path())
        .await
        .with_context(|| format!("reading {}", encrypted_file.path().display()))?;
    if encrypted_meta.len() == 0 {
        bail!("decoded secret is empty");
    }

    decrypt_age_file(
        encrypted_file.path(),
        identities,
        quiet_phone_progress,
        key,
        multiple_phone_identities,
    )
    .await
}

pub(super) async fn preflight_identities(identities: &[PathBuf]) -> Result<()> {
    if identities.is_empty() {
        bail!("no age identities configured");
    }
    preflight_age().await?;
    for plugin in required_identity_plugins(identities).await? {
        ensure_identity_plugin(plugin)?;
    }
    Ok(())
}

pub(super) async fn preflight_recipients(recipients: &[String]) -> Result<()> {
    let recipients = validate_recipients(recipients)?;
    preflight_age().await?;
    preflight_recipient_plugins(&recipients)
}

pub(super) async fn preflight_age() -> Result<()> {
    ensure_command("age")?;
    let output = Command::new("age")
        .arg("--version")
        .output()
        .await
        .context("checking age version")?;
    if !output.status.success() {
        bail!("age 1.3 or newer is required; `age --version` failed");
    }
    let stdout = String::from_utf8(output.stdout).context("age --version output is not UTF-8")?;
    validate_age_version(&stdout)?;
    Ok(())
}

fn validate_age_version(output: &str) -> Result<Version> {
    let version = parse_age_version(output)?;
    let minimum = Version::parse(MINIMUM_AGE_VERSION).expect("minimum age version is valid");
    if version < minimum {
        bail!("age 1.3 or newer is required; found {version}");
    }
    Ok(version)
}

fn parse_age_version(output: &str) -> Result<Version> {
    let raw = output
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_start_matches('v');
    Version::parse(raw).context("could not parse age version; age 1.3 or newer is required")
}

fn preflight_recipient_plugins(recipients: &[&str]) -> Result<()> {
    if recipients
        .iter()
        .any(|recipient| recipient.starts_with("age1se1"))
        && ensure_command("age-plugin-se").is_err()
    {
        bail!(
            "legacy age1se recipient requires age-plugin-se for encryption; with age 1.3 or newer, run `shine state migrate --dry-run` and then `shine state migrate` to convert configured recipients to age1tag"
        );
    }
    if recipients
        .iter()
        .any(|recipient| recipient.starts_with("age1phone1"))
        && ensure_command("age-plugin-phone").is_err()
    {
        bail!(
            "age1phone recipient requires age-plugin-phone on every computer that seals for it; install age-plugin-phone and retry, or upgrade the desktop plugin and phone app to tagged-recipient support, export with `age-plugin-phone recipients -i <IDENTITY_STUB> --recipient-type tag`, verify and replace the configured recipient before resealing; `shine state migrate` does not convert phone recipients"
        );
    }
    Ok(())
}

fn ensure_identity_plugin(plugin: &str) -> Result<()> {
    if plugin == "age-plugin-se" && ensure_command(plugin).is_err() {
        bail!(
            "legacy Secure Enclave identity requires age-plugin-se to decrypt this payload; install the plugin, then upgrade to age 1.3 or newer and run `shine state migrate --dry-run` / `shine state migrate` before resealing"
        );
    }
    ensure_command(plugin)
}

pub(super) fn secure_enclave_recipient_to_tag(recipient: &str) -> Result<String> {
    let (hrp, data, variant) =
        bech32::decode(recipient).context("legacy Secure Enclave recipient is not valid Bech32")?;
    if hrp != "age1se" || variant != Variant::Bech32 {
        bail!("legacy Secure Enclave recipient must use the age1se Bech32 encoding");
    }
    bech32::encode("age1tag", data, Variant::Bech32).context("encoding native tagged age recipient")
}

fn phone_terminal_output_requested(transport: Option<&OsStr>, messages: Option<&OsStr>) -> bool {
    let transport = transport.and_then(OsStr::to_str).map(str::trim);
    let qr_transport = match transport {
        Some(value) if value.eq_ignore_ascii_case("qr") => true,
        Some(value) if value.eq_ignore_ascii_case("auto") => !cfg!(windows),
        None => !cfg!(windows),
        _ => false,
    };
    qr_transport
        || messages.and_then(OsStr::to_str).is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
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

async fn required_identity_plugins(identities: &[PathBuf]) -> Result<Vec<&'static str>> {
    Ok(inspect_identity_plugins(identities).await?.required)
}

#[derive(Debug, PartialEq, Eq)]
struct IdentityPluginInspection {
    required: Vec<&'static str>,
    phone_identity_count: usize,
}

async fn inspect_identity_plugins(identities: &[PathBuf]) -> Result<IdentityPluginInspection> {
    let mut plugins = Vec::new();
    let mut phone_identity_count = 0;
    for identity in identities {
        if !identity.is_file() {
            bail!("age identity file not found: {}", identity.display());
        }
        let contents = tokio::fs::read_to_string(identity)
            .await
            .with_context(|| format!("reading age identity {}", identity.display()))?;
        for line in contents.lines().map(str::trim) {
            if line.starts_with("AGE-PLUGIN-PHONE-") {
                phone_identity_count += 1;
            }
        }
        for (marker, plugin) in [
            ("AGE-PLUGIN-SE-", "age-plugin-se"),
            ("AGE-PLUGIN-PHONE-", "age-plugin-phone"),
        ] {
            if contents.contains(marker) && !plugins.contains(&plugin) {
                plugins.push(plugin);
            }
        }
    }
    Ok(IdentityPluginInspection {
        required: plugins,
        phone_identity_count,
    })
}

async fn decrypt_age_file(
    path: &Path,
    identities: &[PathBuf],
    quiet_phone_progress: bool,
    key: bool,
    multiple_phone_identities: bool,
) -> Result<String> {
    let mut command = Command::new("age");
    command.kill_on_drop(true).arg("-d");
    for identity in identities {
        command.arg("-i").arg(identity);
    }
    command.arg(path);

    let output = command
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(if quiet_phone_progress {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::inherit()
        })
        .spawn()
        .with_context(|| "running age -d")?;

    if key {
        return contextualize_hybrid_key_error(
            super::exec::read_key_output(output).await,
            multiple_phone_identities,
        );
    }
    let output = output
        .wait_with_output()
        .await
        .context("waiting for age -d")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = stderr.trim();
        if !diagnostic.is_empty() {
            bail!("age decrypt failed: {diagnostic}");
        }
        bail!("age decrypt failed");
    }

    String::from_utf8(output.stdout).context("decrypted secret is not valid UTF-8")
}

fn contextualize_hybrid_key_error(
    result: Result<String>,
    multiple_phone_identities: bool,
) -> Result<String> {
    if multiple_phone_identities {
        return result.context(
            "multiple phone-backed age identities are configured; if an older age-plugin-phone stops at a nonmatching identity, upgrade the plugin or temporarily configure only the matching identity",
        );
    }
    result
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

    #[test]
    fn parses_supported_age_versions() {
        assert_eq!(
            parse_age_version("v1.3.2\n").unwrap(),
            Version::new(1, 3, 2)
        );
        assert_eq!(
            parse_age_version("age v2.0.0\n").unwrap(),
            Version::new(2, 0, 0)
        );
        assert!(parse_age_version("age unknown\n").is_err());
        assert!(validate_age_version("v1.2.1\n").is_err());
        assert_eq!(
            validate_age_version("v1.3.0\n").unwrap(),
            Version::new(1, 3, 0)
        );
    }

    #[test]
    fn converts_secure_enclave_recipient_to_native_tag() {
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let tagged = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
        let converted = secure_enclave_recipient_to_tag(legacy).unwrap();
        assert_eq!(converted, tagged);
        let (_, legacy_data, _) = bech32::decode(legacy).unwrap();
        let (_, tagged_data, _) = bech32::decode(&converted).unwrap();
        assert_eq!(legacy_data, tagged_data);
    }

    #[test]
    fn rejects_invalid_secure_enclave_recipient() {
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let tagged = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
        let mut bad_checksum = legacy.to_string();
        bad_checksum.pop();
        bad_checksum.push('q');

        assert!(secure_enclave_recipient_to_tag("age1se1invalid").is_err());
        assert!(secure_enclave_recipient_to_tag(&bad_checksum).is_err());
        assert!(secure_enclave_recipient_to_tag(tagged).is_err());
    }

    #[test]
    fn tagged_recipients_need_no_secure_enclave_plugin() {
        preflight_recipient_plugins(&["age1tag1example"]).unwrap();
    }

    #[test]
    fn missing_known_recipient_plugins_have_actionable_errors() {
        let _guard = crate::test_support::env_lock();
        let old_path = std::env::var_os("PATH");
        // SAFETY: env_lock serializes process-environment mutation tests.
        unsafe {
            std::env::set_var(
                "PATH",
                std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            )
        };

        let se = preflight_recipient_plugins(&["age1se1example"]).unwrap_err();
        let phone = preflight_recipient_plugins(&["age1phone1example"]).unwrap_err();

        // SAFETY: same env_lock guard as above.
        unsafe {
            match old_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
        assert!(se.to_string().contains("shine state migrate --dry-run"));
        assert!(phone.to_string().contains("every computer that seals"));
    }

    #[test]
    fn missing_legacy_identity_plugin_has_migration_guidance() {
        let _guard = crate::test_support::env_lock();
        let old_path = std::env::var_os("PATH");
        // SAFETY: env_lock serializes process-environment mutation tests.
        unsafe {
            std::env::set_var(
                "PATH",
                std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            )
        };

        let err = ensure_identity_plugin("age-plugin-se").unwrap_err();

        // SAFETY: same env_lock guard as above.
        unsafe {
            match old_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
        assert!(err.to_string().contains("requires age-plugin-se"));
        assert!(err.to_string().contains("shine state migrate --dry-run"));
        assert!(err.to_string().contains("before resealing"));
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

    #[test]
    fn phone_terminal_output_is_explicit_or_required_by_qr() {
        assert_eq!(phone_terminal_output_requested(None, None), !cfg!(windows));
        assert_eq!(
            phone_terminal_output_requested(Some(OsStr::new("auto")), None),
            !cfg!(windows)
        );
        assert!(!phone_terminal_output_requested(
            Some(OsStr::new("adb")),
            Some(OsStr::new("0")),
        ));
        assert!(phone_terminal_output_requested(
            Some(OsStr::new("qr")),
            None,
        ));
        assert!(phone_terminal_output_requested(
            Some(OsStr::new("wifi")),
            Some(OsStr::new("true")),
        ));
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

    #[tokio::test]
    async fn detects_each_supported_identity_plugin_once() {
        let dir = std::env::temp_dir().join(format!("shine-age-plugins-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let secure_enclave = dir.join("secure-enclave.txt");
        let phone = dir.join("phone.txt");
        tokio::fs::write(&secure_enclave, "AGE-PLUGIN-SE-1EXAMPLE\n")
            .await
            .unwrap();
        tokio::fs::write(
            &phone,
            "AGE-PLUGIN-PHONE-1EXAMPLE\nAGE-PLUGIN-PHONE-1DUPLICATE\n",
        )
        .await
        .unwrap();

        let inspection = inspect_identity_plugins(&[secure_enclave, phone])
            .await
            .unwrap();
        assert_eq!(
            inspection.required,
            vec!["age-plugin-se", "age-plugin-phone"]
        );
        assert_eq!(inspection.phone_identity_count, 2);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn hybrid_failure_explains_multiple_phone_identity_compatibility() {
        let err =
            contextualize_hybrid_key_error(Err(anyhow::anyhow!("hybrid unwrap failed")), true)
                .unwrap_err();
        let diagnostic = format!("{err:#}");
        assert!(diagnostic.contains("multiple phone-backed age identities"));
        assert!(diagnostic.contains("upgrade the plugin"));
        assert!(diagnostic.contains("hybrid unwrap failed"));
    }
}
