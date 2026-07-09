//! Shared subprocess helpers for shelling out to external secret-handling
//! CLIs (`gpg`, `age`, `base64`). Used by both `secret::gpg` and
//! `secret::age` so their process-spawning conventions stay identical.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub(crate) fn ensure_command(name: &str) -> Result<()> {
    let found = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                if dir.join(name).is_file() {
                    return true;
                }
                #[cfg(windows)]
                if dir.join(format!("{name}.exe")).is_file() {
                    return true;
                }
                false
            })
        })
        .unwrap_or(false);
    if !found {
        bail!("{name} is not installed or not on PATH");
    }
    Ok(())
}

pub(crate) async fn decode_base64_to_file(encoded_secret: &str, output_path: &Path) -> Result<()> {
    if run_base64_decode(encoded_secret, output_path, "--decode").await? {
        return Ok(());
    }
    if run_base64_decode(encoded_secret, output_path, "-D").await? {
        return Ok(());
    }
    bail!("secret is not valid base64");
}

async fn run_base64_decode(encoded_secret: &str, output_path: &Path, flag: &str) -> Result<bool> {
    let output = Command::new("base64")
        .arg(flag)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("running base64 {flag}"))?;

    let output = write_stdin_and_wait(output, encoded_secret.as_bytes()).await?;
    if output.status.success() {
        tokio::fs::write(output_path, output.stdout)
            .await
            .with_context(|| format!("writing {}", output_path.display()))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(crate) async fn encode_base64_single_line(input: &[u8]) -> Result<String> {
    let output = Command::new("base64")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| "running base64")?;

    let output = write_stdin_and_wait(output, input).await?;
    if !output.status.success() {
        bail!("base64 encode failed");
    }

    let encoded = String::from_utf8(output.stdout).context("base64 output is not valid UTF-8")?;
    Ok(encoded.split_whitespace().collect())
}

pub(crate) async fn write_stdin_and_wait(
    mut child: tokio::process::Child,
    input: &[u8],
) -> Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;

    let mut stdin = child.stdin.take().context("opening child stdin")?;
    stdin
        .write_all(input)
        .await
        .context("writing child stdin")?;
    drop(stdin);

    child
        .wait_with_output()
        .await
        .context("waiting for child process")
}

pub(crate) struct TempFile {
    path: PathBuf,
}

impl TempFile {
    /// Creates the temp file with owner-only (`0600`) permissions on Unix,
    /// set atomically at open time rather than via a follow-up `chmod` — the
    /// file briefly holds ciphertext, so it should never inherit the
    /// process umask's default (typically world-readable `0644`) in a
    /// shared `/tmp`.
    pub(crate) async fn new(prefix: &str) -> Result<Self> {
        let mut path = std::env::temp_dir();
        path.push(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        options
            .open(&path)
            .await
            .with_context(|| format!("creating {}", path.display()))?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
