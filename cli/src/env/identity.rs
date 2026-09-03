//! `shine env secret identity init/list`: generate and inspect age identities used
//! to decrypt `age:`-tagged secrets, including Secure Enclave (Touch ID)
//! identities minted by `age-plugin-se` or paired through `age-plugin-phone`.

use anyhow::{Context, Result, bail};
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
    ensure_command("age-plugin-phone")?;
    let label = resolve_phone_label(label)?;
    let result = run_phone_setup("age-plugin-phone", &label, transport, adb_serial).await?;
    validate_phone_setup_result(&result).await?;

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
            &[
                "keygen".to_string(),
                format!("--access-control={access_control}"),
                "-o".to_string(),
                output_path.to_string_lossy().into_owned(),
            ],
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
    if os != "windows" {
        bail!(
            "phone-backed identity setup currently requires the Windows Alpha platform; use age-plugin-phone directly for diagnostic interoperability on other platforms"
        );
    }
    Ok(())
}

fn resolve_phone_label(explicit: Option<&str>) -> Result<String> {
    let label = explicit.map(str::to_owned).unwrap_or_else(|| {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|value| {
                let trimmed = value.trim();
                !trimmed.is_empty() && trimmed.len() <= 64
            })
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
        bail!("age-plugin-phone setup failed");
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

async fn validate_phone_setup_result(result: &PhoneSetupResult) -> Result<()> {
    if result.schema_version != PHONE_SETUP_RESULT_VERSION {
        bail!(
            "unsupported age-plugin-phone setup result version {}",
            result.schema_version
        );
    }
    if !result.recipient.starts_with("age1phone")
        || result.recipient.chars().any(char::is_whitespace)
    {
        bail!("age-plugin-phone returned an invalid recipient");
    }
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

    #[test]
    fn phone_setup_requires_windows() {
        assert!(ensure_phone_supported("windows").is_ok());
        let err = ensure_phone_supported("macos").unwrap_err();
        assert!(err.to_string().contains("Windows Alpha"), "{err:#}");
    }

    #[test]
    fn explicit_phone_label_uses_the_plugin_byte_limit() {
        assert_eq!(
            resolve_phone_label(Some("Work laptop")).unwrap(),
            "Work laptop"
        );
        assert!(resolve_phone_label(Some(" ")).is_err());
        assert!(resolve_phone_label(Some(&"桌".repeat(22))).is_err());
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
            "# public age-plugin-phone identity stub\n# recipient: age1phone1example\nAGE-PLUGIN-PHONE-1EXAMPLE\n",
        )
        .await
        .unwrap();
        let result = PhoneSetupResult {
            schema_version: PHONE_SETUP_RESULT_VERSION,
            identity_path: path,
            recipient: "age1phone1example".to_string(),
        };

        validate_phone_setup_result(&result).await.unwrap();
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
            recipient: "age1phone1different".to_string(),
        };

        let err = validate_phone_setup_result(&result).await.unwrap_err();
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
        tokio::fs::write(
            &program,
            concat!(
                "#!/bin/sh\n",
                "test \"$1\" = setup || exit 11\n",
                "test \"$2\" = --label || exit 12\n",
                "test \"$3\" = 'Work laptop' || exit 13\n",
                "test \"$4\" = --transport || exit 14\n",
                "test \"$5\" = qr || exit 15\n",
                "test \"$6\" = --json || exit 16\n",
                "test \"$#\" = 6 || exit 17\n",
                "printf '%s\\n' '{\"schema_version\":1,\"identity_path\":\"/tmp/phone-identity.txt\",\"recipient\":\"age1phone1example\"}'\n",
            ),
        )
        .await
        .unwrap();
        let mut permissions = tokio::fs::metadata(&program).await.unwrap().permissions();
        permissions.set_mode(0o700);
        tokio::fs::set_permissions(&program, permissions)
            .await
            .unwrap();

        let result = run_phone_setup(program.to_str().unwrap(), "Work laptop", "qr", None)
            .await
            .unwrap();
        assert_eq!(result.schema_version, PHONE_SETUP_RESULT_VERSION);
        assert_eq!(
            result.identity_path,
            PathBuf::from("/tmp/phone-identity.txt")
        );
        assert_eq!(result.recipient, "age1phone1example");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
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
