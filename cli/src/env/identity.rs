//! `shine env secret identity init/list`: generate and inspect age identities used
//! to decrypt `age:`-tagged secrets, including Secure Enclave (Touch ID)
//! identities minted by `age-plugin-se` or paired through `age-plugin-phone`.

use crate::commands::PhoneRecipientType;
use anyhow::{Context, Result, bail};
use bech32::{FromBase32, Variant};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::Config;
use crate::proc::ensure_command;
use crate::{colors, path_display};

const DEFAULT_ACCESS_CONTROL: &str = "any-biometry";
const VALID_ACCESS_CONTROLS: &[&str] = &[
    "any-biometry",
    "any-biometry-or-passcode",
    "current-biometry",
    "passcode",
];
const PHONE_SETUP_RESULT_VERSION: u16 = 1;
const MAX_PHONE_SETUP_RESULT_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhoneSetupResult {
    schema_version: u16,
    identity_path: PathBuf,
    recipient: String,
}

#[derive(Serialize)]
struct ManualAgeIdentities<'a> {
    age_identities: &'a [String],
}

pub async fn handle_phone_identity_init(
    config: &Config,
    recipient_type: PhoneRecipientType,
    label: Option<&str>,
    transport: &str,
    adb_serial: Option<&str>,
) -> Result<()> {
    ensure_phone_supported(std::env::consts::OS)?;
    if config.project_overrides_age_identities() {
        bail!(
            "the active project explicitly overrides age identity configuration; remove or update that project override before pairing a phone-backed identity"
        );
    }
    if recipient_type == PhoneRecipientType::Tag {
        crate::secret::preflight_age().await?;
    }
    ensure_command("age-plugin-phone")?;
    let system_label = if label.is_none() {
        read_phone_computer_name().await
    } else {
        None
    };
    let label = resolve_phone_label(label, system_label.as_deref())?;
    let result = run_phone_setup(
        "age-plugin-phone",
        recipient_type,
        &label,
        transport,
        adb_serial,
    )
    .await?;
    validate_phone_setup_result(&result, recipient_type).await?;

    let identity_value = result
        .identity_path
        .to_str()
        .context("phone identity path is not valid Unicode")?
        .to_owned();
    let mut global = Config::load_global_runtime_for_dry_run().await?;
    add_age_identity_path(&mut global, &result.identity_path, identity_value);
    if let Err(error) = global.save().await {
        let manual = toml::to_string(&ManualAgeIdentities {
            age_identities: &global.age_identities,
        })
        .unwrap_or_else(|_| "age_identities = [\"<phone identity path>\"]\n".to_string());
        eprintln!(
            "Phone pairing succeeded, but Shine could not update {}. The pairing remains active; do not start another setup. Add this to the global config manually:\n\n{}",
            path_display::format(global.config_path()),
            manual.trim_end()
        );
        return Err(error).context("saving the phone identity in global Shine config");
    }

    println!(
        "{}",
        colors::green(&format!(
            "configured phone-backed age identity at {}",
            path_display::format(&result.identity_path)
        ))
    );
    println!("  recipient: {}", result.recipient);
    println!(
        "  global config: {}",
        path_display::format(global.config_path())
    );
    println!();
    println!(
        "{}",
        colors::dim(
            "Add this phone recipient together with an independently verified recovery recipient to age_recipients or the workspace [env.encryption] table. Never use the preview phone recipient as the only recipient for retained data."
        )
    );
    if global.secret_backend.as_deref() != Some("age") {
        println!(
            "{}",
            colors::dim(
                "The global secret_backend was not changed. Set secret_backend = \"age\" explicitly if age should become the default for encrypt/seal."
            )
        );
    }
    Ok(())
}

pub async fn handle_identity_init(
    config: &Config,
    touch_id: bool,
    access_control: Option<&str>,
    output: Option<&Path>,
    force: bool,
) -> Result<()> {
    ensure_touch_id_supported(touch_id, std::env::consts::OS)?;
    if !touch_id && access_control.is_some() {
        bail!("--access-control only applies with --touch-id");
    }
    let access_control = access_control.unwrap_or(DEFAULT_ACCESS_CONTROL);
    if touch_id {
        validate_access_control(access_control)?;
    }

    let output_path = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_identity_path(config));
    if output_path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            output_path.display()
        );
    }
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    if touch_id {
        ensure_command("age-plugin-se")?;
        run_keygen(
            "age-plugin-se",
            &touch_id_keygen_args(access_control, &output_path),
        )
        .await?;
    } else {
        ensure_command("age-keygen")?;
        run_keygen(
            "age-keygen",
            &["-o".to_string(), output_path.to_string_lossy().into_owned()],
        )
        .await?;
    }

    #[cfg(unix)]
    set_owner_only_permissions(&output_path).await?;

    let recipient = extract_recipient(&output_path).await?;
    println!(
        "{}",
        colors::green(&format!(
            "generated age identity at {}",
            path_display::format(&output_path)
        ))
    );
    println!("  recipient: {recipient}");
    println!();
    println!(
        "{}",
        colors::dim(
            "Add this recipient to age_recipients in config.toml (or [env.encryption] in \
             shine.workspace.toml) so others can decrypt secrets sealed for it."
        )
    );
    if config.secret_backend.as_deref() != Some("age") {
        println!(
            "{}",
            colors::dim(
                "Set secret_backend = \"age\" in config.toml to make age the default for \
                 `shine env secret encrypt`/`shine env secret seal`."
            )
        );
    }
    if config.age_identity.is_none() && output_path != default_identity_path(config) {
        println!(
            "{}",
            colors::dim(&format!(
                "Set age_identity = \"{}\" in config.toml so shine can find this identity.",
                output_path.display()
            ))
        );
    }
    Ok(())
}

fn touch_id_keygen_args(access_control: &str, output_path: &Path) -> Vec<String> {
    vec![
        "keygen".to_string(),
        "--recipient-type=tag".to_string(),
        format!("--access-control={access_control}"),
        "-o".to_string(),
        output_path.to_string_lossy().into_owned(),
    ]
}

pub async fn handle_identity_list(config: &Config) -> Result<()> {
    let identities = config.resolved_age_identities();
    if identities.is_empty() {
        println!(
            "{}",
            colors::dim("No age identity configured. Run `shine env secret identity init`.")
        );
        return Ok(());
    }
    for identity in &identities {
        let recipient = extract_recipient(identity).await?;
        println!("{}  {}", path_display::format(identity), recipient);
    }
    Ok(())
}

fn ensure_phone_supported(os: &str) -> Result<()> {
    if !matches!(os, "windows" | "macos") {
        bail!(
            "phone-backed identity setup requires Windows or macOS (experimental); use age-plugin-phone directly for diagnostic interoperability on other platforms"
        );
    }
    Ok(())
}

async fn read_phone_computer_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/sbin/scutil")
            .args(["--get", "ComputerName"])
            .stdin(Stdio::null())
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8(output.stdout).ok()?.trim_end().to_owned())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

fn resolve_phone_label(explicit: Option<&str>, system_label: Option<&str>) -> Result<String> {
    let label = explicit.map(str::to_owned).unwrap_or_else(|| {
        system_label
            .filter(|value| !value.trim().is_empty() && value.len() <= 64)
            .map(str::to_owned)
            .unwrap_or_else(|| "Shine desktop".to_string())
    });
    if label.trim().is_empty() {
        bail!("--label must not be empty");
    }
    if label.len() > 64 {
        bail!("--label must be at most 64 UTF-8 bytes");
    }
    Ok(label)
}

async fn run_phone_setup(
    program: &str,
    recipient_type: PhoneRecipientType,
    label: &str,
    transport: &str,
    adb_serial: Option<&str>,
) -> Result<PhoneSetupResult> {
    let mut command = Command::new(program);
    command
        .arg("setup")
        .arg("--label")
        .arg(label)
        .arg("--transport")
        .arg(transport)
        .arg("--json")
        .arg("--recipient-type")
        .arg(recipient_type.as_str())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(serial) = adb_serial {
        command.arg("--adb-serial").arg(serial);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("running {program} setup"))?;
    let stdout = child
        .stdout
        .take()
        .context("capturing age-plugin-phone setup result")?;
    let (status, bytes) = tokio::join!(child.wait(), read_bounded_output(stdout));
    let status = status.context("waiting for age-plugin-phone setup")?;
    let bytes = bytes?;
    if !status.success() {
        bail!(
            "age-plugin-phone setup failed; use a plugin version supporting --recipient-type and a matching phone app. No fallback or second setup was attempted"
        );
    }
    serde_json::from_slice(&bytes).context("invalid age-plugin-phone setup result")
}

async fn read_bounded_output(mut stdout: tokio::process::ChildStdout) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut overflow = false;
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stdout
            .read(&mut chunk)
            .await
            .context("reading age-plugin-phone setup result")?;
        if read == 0 {
            break;
        }
        if output.len().saturating_add(read) <= MAX_PHONE_SETUP_RESULT_BYTES {
            output.extend_from_slice(&chunk[..read]);
        } else {
            overflow = true;
        }
    }
    if overflow {
        bail!("age-plugin-phone setup result exceeded the size limit");
    }
    Ok(output)
}

async fn validate_phone_setup_result(
    result: &PhoneSetupResult,
    recipient_type: PhoneRecipientType,
) -> Result<()> {
    if result.schema_version != PHONE_SETUP_RESULT_VERSION {
        bail!(
            "unsupported age-plugin-phone setup result version {}",
            result.schema_version
        );
    }
    validate_phone_recipient(&result.recipient, recipient_type)?;
    if !result.identity_path.is_absolute() {
        bail!("age-plugin-phone returned a non-absolute identity path");
    }
    let metadata = tokio::fs::symlink_metadata(&result.identity_path)
        .await
        .with_context(|| format!("reading {}", result.identity_path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("age-plugin-phone identity stub is not a regular file");
    }
    let contents = tokio::fs::read_to_string(&result.identity_path)
        .await
        .with_context(|| format!("reading {}", result.identity_path.display()))?;
    if !contents
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("AGE-PLUGIN-PHONE-"))
    {
        bail!("age-plugin-phone identity stub has no phone plugin identity");
    }
    let recipient = extract_recipient(&result.identity_path).await?;
    if recipient != result.recipient {
        bail!("age-plugin-phone setup result does not match its identity stub");
    }
    Ok(())
}

fn validate_phone_recipient(recipient: &str, recipient_type: PhoneRecipientType) -> Result<()> {
    let (hrp, data, variant) = bech32::decode(recipient)
        .context("age-plugin-phone returned an invalid Bech32 recipient")?;
    let expected = match recipient_type {
        PhoneRecipientType::Tag => "age1tag",
        PhoneRecipientType::Phone => "age1phone",
    };
    if hrp != expected || variant != Variant::Bech32 {
        bail!("age-plugin-phone returned a recipient that does not match the requested type");
    }
    let bytes = Vec::<u8>::from_base32(&data).context("invalid phone recipient payload")?;
    if bech32::encode(&hrp, data, variant)? != recipient {
        bail!("age-plugin-phone returned a non-canonical recipient");
    }
    if bytes.is_empty()
        || (recipient_type == PhoneRecipientType::Tag
            && (bytes.len() != 33 || !matches!(bytes[0], 2 | 3)))
    {
        bail!("age-plugin-phone returned an invalid recipient public-key payload");
    }
    Ok(())
}

fn add_age_identity_path(config: &mut Config, path: &Path, value: String) {
    if config.age_identity.is_none() && config.age_identities.is_empty() {
        let implicit = config
            .resolved_age_identities()
            .into_iter()
            .filter_map(|existing| existing.to_str().map(str::to_owned))
            .collect::<Vec<_>>();
        config.age_identities.extend(implicit);
    }
    if !config
        .resolved_age_identities()
        .iter()
        .any(|item| item == path)
    {
        config.age_identities.push(value);
    }
}

fn ensure_touch_id_supported(touch_id: bool, os: &str) -> Result<()> {
    if touch_id && os != "macos" {
        bail!(
            "Secure Enclave identities require macOS; run `shine env secret identity init` without \
             --touch-id to generate a plain age identity"
        );
    }
    Ok(())
}

fn validate_access_control(value: &str) -> Result<()> {
    if !VALID_ACCESS_CONTROLS.contains(&value) {
        bail!(
            "unknown --access-control \"{value}\"; expected one of: {}",
            VALID_ACCESS_CONTROLS.join(", ")
        );
    }
    Ok(())
}

fn default_identity_path(config: &Config) -> PathBuf {
    config.shine_dir().join("age").join("identity.txt")
}

async fn run_keygen(program: &str, args: &[String]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .await
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} failed");
    }
    Ok(())
}

#[cfg(unix)]
async fn set_owner_only_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o600);
    tokio::fs::set_permissions(path, permissions)
        .await
        .with_context(|| format!("setting permissions on {}", path.display()))
}

/// Extract an `age1...` recipient from an identity file's leading comment, as
/// written by native keygen and supported hardware plugins.
async fn extract_recipient(path: &Path) -> Result<String> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    contents
        .lines()
        .filter_map(|line| line.strip_prefix('#'))
        .map(str::trim)
        .find_map(|line| {
            line.split_whitespace()
                .find(|token| token.starts_with("age1"))
        })
        .map(str::to_string)
        .with_context(|| format!("no recipient found in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_id_requires_macos() {
        let err = ensure_touch_id_supported(true, "linux").unwrap_err();
        assert!(err.to_string().contains("require macOS"), "{err:#}");
    }

    #[test]
    fn touch_id_allowed_on_macos() {
        assert!(ensure_touch_id_supported(true, "macos").is_ok());
    }

    #[test]
    fn non_touch_id_allowed_on_any_os() {
        assert!(ensure_touch_id_supported(false, "linux").is_ok());
        assert!(ensure_touch_id_supported(false, "windows").is_ok());
    }

    const TAG: &str = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
    const PHONE: &str = "age1phone1qypkk9737tsjcsj8lz7wdetr53q0yacr0kqjm6en5r62zw29mzvv99sa27n9c";

    #[test]
    fn phone_recipient_validation_checks_type_encoding_and_tag_structure() {
        use bech32::ToBase32;
        validate_phone_recipient(TAG, PhoneRecipientType::Tag).unwrap();
        validate_phone_recipient(PHONE, PhoneRecipientType::Phone).unwrap();
        for (recipient, kind) in [
            (PHONE, PhoneRecipientType::Tag),
            (TAG, PhoneRecipientType::Phone),
            ("age1tag1invalid", PhoneRecipientType::Tag),
            ("age1phone1invalid", PhoneRecipientType::Phone),
            (&TAG.to_uppercase(), PhoneRecipientType::Tag),
        ] {
            assert!(validate_phone_recipient(recipient, kind).is_err());
        }
        for bytes in [vec![], vec![2; 32], vec![2; 34], vec![4; 33]] {
            let invalid = bech32::encode("age1tag", bytes.to_base32(), Variant::Bech32).unwrap();
            assert!(validate_phone_recipient(&invalid, PhoneRecipientType::Tag).is_err());
        }
        let wrong_variant =
            bech32::encode("age1tag", vec![2; 33].to_base32(), Variant::Bech32m).unwrap();
        assert!(validate_phone_recipient(&wrong_variant, PhoneRecipientType::Tag).is_err());
    }

    #[tokio::test]
    async fn tagged_phone_setup_keeps_the_phone_identity_stub() {
        let dir = std::env::temp_dir().join(format!("shine-phone-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("phone.txt");
        let content = format!("# recipient: {TAG}\nAGE-PLUGIN-PHONE-1EXAMPLE\n");
        tokio::fs::write(&path, &content).await.unwrap();
        let result = PhoneSetupResult {
            schema_version: 1,
            identity_path: path.clone(),
            recipient: TAG.into(),
        };
        validate_phone_setup_result(&result, PhoneRecipientType::Tag)
            .await
            .unwrap();
        assert!(
            validate_phone_setup_result(&result, PhoneRecipientType::Phone)
                .await
                .is_err()
        );
        assert_eq!(extract_recipient(&path).await.unwrap(), TAG);
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), content);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn phone_setup_allows_windows_and_macos_only() {
        assert!(ensure_phone_supported("windows").is_ok());
        assert!(ensure_phone_supported("macos").is_ok());
        for os in ["linux", "unknown"] {
            let err = ensure_phone_supported(os).unwrap_err();
            assert!(err.to_string().contains("Windows or macOS"), "{err:#}");
        }
    }

    #[test]
    fn explicit_phone_label_uses_the_plugin_byte_limit() {
        assert_eq!(
            resolve_phone_label(Some("Work laptop"), Some("System name")).unwrap(),
            "Work laptop"
        );
        assert!(resolve_phone_label(Some(" "), Some("System name")).is_err());
        assert!(resolve_phone_label(Some(&"桌".repeat(22)), None).is_err());
        assert!(resolve_phone_label(Some(&"a".repeat(64)), None).is_ok());
    }

    #[test]
    fn phone_label_uses_a_valid_system_name_or_fallback() {
        for name in ["Work Mac", "工作电脑", &"a".repeat(64)] {
            assert_eq!(resolve_phone_label(None, Some(name)).unwrap(), name);
        }
        for name in [
            None,
            Some(""),
            Some("  "),
            Some(&"桌".repeat(22)),
            Some(&format!(" {} ", "a".repeat(64))),
        ] {
            assert_eq!(resolve_phone_label(None, name).unwrap(), "Shine desktop");
        }
    }

    #[test]
    fn access_control_validates_known_values() {
        for value in VALID_ACCESS_CONTROLS {
            assert!(validate_access_control(value).is_ok());
        }
        let err = validate_access_control("bogus").unwrap_err();
        assert!(
            err.to_string().contains("unknown --access-control"),
            "{err:#}"
        );
    }

    #[test]
    fn touch_id_keygen_requests_native_tagged_recipient() {
        let args = touch_id_keygen_args("any-biometry", Path::new("identity.txt"));
        assert_eq!(
            args,
            [
                "keygen",
                "--recipient-type=tag",
                "--access-control=any-biometry",
                "-o",
                "identity.txt",
            ]
        );
    }

    #[tokio::test]
    async fn extracts_recipient_from_identity_comment() {
        let dir = std::env::temp_dir().join(format!("shine-identity-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("identity.txt");
        tokio::fs::write(
            &path,
            "# created: 2026-01-01\n# public key: age1qexampleexampleexample\nAGE-SECRET-KEY-1EXAMPLE\n",
        )
        .await
        .unwrap();

        let recipient = extract_recipient(&path).await.unwrap();
        assert_eq!(recipient, "age1qexampleexampleexample");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extract_recipient_errors_without_recipient_comment() {
        let dir = std::env::temp_dir().join(format!("shine-identity-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("identity.txt");
        tokio::fs::write(&path, "AGE-SECRET-KEY-1EXAMPLE\n")
            .await
            .unwrap();

        let err = extract_recipient(&path).await.unwrap_err();
        assert!(err.to_string().contains("no recipient found"), "{err:#}");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn validates_matching_phone_setup_result() {
        let dir =
            std::env::temp_dir().join(format!("shine-phone-identity-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("identity.txt");
        tokio::fs::write(
            &path,
            "# public age-plugin-phone identity stub\n# recipient: age1phone1qypkk9737tsjcsj8lz7wdetr53q0yacr0kqjm6en5r62zw29mzvv99sa27n9c\nAGE-PLUGIN-PHONE-1EXAMPLE\n",
        )
        .await
        .unwrap();
        let result = PhoneSetupResult {
            schema_version: PHONE_SETUP_RESULT_VERSION,
            identity_path: path,
            recipient: "age1phone1qypkk9737tsjcsj8lz7wdetr53q0yacr0kqjm6en5r62zw29mzvv99sa27n9c"
                .to_string(),
        };

        validate_phone_setup_result(&result, PhoneRecipientType::Phone)
            .await
            .unwrap();
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_phone_setup_result_that_disagrees_with_stub() {
        let dir =
            std::env::temp_dir().join(format!("shine-phone-identity-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("identity.txt");
        tokio::fs::write(
            &path,
            "# recipient: age1phone1actual\nAGE-PLUGIN-PHONE-1EXAMPLE\n",
        )
        .await
        .unwrap();
        let result = PhoneSetupResult {
            schema_version: PHONE_SETUP_RESULT_VERSION,
            identity_path: path,
            recipient: "age1phone1qypkk9737tsjcsj8lz7wdetr53q0yacr0kqjm6en5r62zw29mzvv99sa27n9c"
                .to_string(),
        };

        let err = validate_phone_setup_result(&result, PhoneRecipientType::Phone)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err:#}");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn phone_setup_uses_the_versioned_plugin_handoff() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir =
            std::env::temp_dir().join(format!("shine-phone-command-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let program = dir.join("fake-age-plugin-phone");
        for (kind, recipient) in [
            (PhoneRecipientType::Tag, TAG),
            (PhoneRecipientType::Phone, PHONE),
        ] {
            let result_json = serde_json::json!({"schema_version": 1, "identity_path": "/tmp/phone-identity.txt", "recipient": recipient});
            let script = format!(
                r#"#!/bin/sh
[ "$1" = setup ] && [ "$2" = --label ] && [ "$3" = 'Work laptop' ] || exit 11
[ "$4" = --transport ] && [ "$5" = qr ] && [ "$6" = --json ] || exit 12
[ "$7" = --recipient-type ] && [ "$8" = {} ] || exit 13
[ "$9" = --adb-serial ] && [ "${{10}}" = 'device serial' ] && [ "$#" = 10 ] || exit 14
printf '%s\n' '{}'
"#,
                kind.as_str(),
                result_json
            );
            tokio::fs::write(&program, script).await.unwrap();
            tokio::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
                .await
                .unwrap();
            let result = run_phone_setup(
                program.to_str().unwrap(),
                kind,
                "Work laptop",
                "qr",
                Some("device serial"),
            )
            .await
            .unwrap();
            assert_eq!(result.schema_version, PHONE_SETUP_RESULT_VERSION);
            assert_eq!(
                result.identity_path,
                PathBuf::from("/tmp/phone-identity.txt")
            );
            assert_eq!(result.recipient, recipient);
        }
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unsupported_phone_setup_never_retries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("shine-phone-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let program = dir.join("old-plugin");
        let calls = dir.join("calls");
        tokio::fs::write(
            &program,
            format!("#!/bin/sh\nprintf x >> '{}'\nexit 2\n", calls.display()),
        )
        .await
        .unwrap();
        tokio::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
        let err = run_phone_setup(
            program.to_str().unwrap(),
            PhoneRecipientType::Tag,
            "test",
            "auto",
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--recipient-type"));
        assert_eq!(tokio::fs::read_to_string(calls).await.unwrap(), "x");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn adding_phone_identity_preserves_implicit_default_identity() {
        let dir =
            std::env::temp_dir().join(format!("shine-phone-identity-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        let default_path = dir.join("age").join("identity.txt");
        let phone_path = dir.join("phone.txt");
        tokio::fs::create_dir_all(default_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&default_path, "AGE-SECRET-KEY-1EXAMPLE\n")
            .await
            .unwrap();

        add_age_identity_path(
            &mut config,
            &phone_path,
            phone_path.to_string_lossy().into_owned(),
        );
        assert_eq!(
            config.resolved_age_identities(),
            vec![default_path, phone_path]
        );
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn default_identity_path_is_under_shine_dir() {
        let dir = std::env::temp_dir().join(format!("shine-identity-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&dir);

        assert_eq!(
            default_identity_path(&config),
            dir.join("age").join("identity.txt")
        );
    }
}
