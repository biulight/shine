use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub(crate) async fn decrypt_base64_gpg_secret(encoded_secret: &str) -> Result<String> {
    if encoded_secret.trim().is_empty() {
        bail!("secret is empty");
    }

    ensure_command("base64").await?;
    ensure_command("gpg").await?;

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

async fn ensure_command(name: &str) -> Result<()> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .status()
        .await
        .with_context(|| format!("checking for {name}"))?;
    if !status.success() {
        bail!("{name} is not installed");
    }
    Ok(())
}

async fn decode_base64_to_file(encoded_secret: &str, output_path: &Path) -> Result<()> {
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

async fn decrypt_gpg_file(path: &Path) -> Result<String> {
    let output = Command::new("gpg")
        .arg("--decrypt")
        .arg(path)
        .stderr(std::process::Stdio::inherit())
        .output()
        .await
        .with_context(|| "running gpg --decrypt")?;
    if !output.status.success() {
        bail!("gpg decrypt failed");
    }

    String::from_utf8(output.stdout).context("decrypted secret is not valid UTF-8")
}

async fn write_stdin_and_wait(
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

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    async fn new(prefix: &str) -> Result<Self> {
        let mut path = std::env::temp_dir();
        path.push(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .with_context(|| format!("creating {}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_secret_fails_before_external_commands() {
        let err = decrypt_base64_gpg_secret("").await.unwrap_err();
        assert!(err.to_string().contains("secret is empty"), "{err:#}");
    }
}
