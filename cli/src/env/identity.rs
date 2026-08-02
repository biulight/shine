//! `shine env secret identity init/list`: generate and inspect age identities used
//! to decrypt `age:`-tagged secrets, including Secure Enclave (Touch ID)
//! identities minted by `age-plugin-se`.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
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
    let identities = config.age_identities();
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

/// Extract the `age1...`/`age1se1...` recipient from an identity file's
/// leading comment, as written by `age-keygen`/`age-plugin-se keygen`.
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
