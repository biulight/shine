//! Shared ciphertext encoding, temporary files, and subprocess helpers for
//! the external `gpg` and `age` CLIs. Base64 is handled in process.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::path::{Path, PathBuf};

pub(crate) async fn decode_base64_to_file(encoded_secret: &str, output_path: &Path) -> Result<()> {
    // Accept wrapped/copy-pasted ciphertext, but never ignore non-whitespace garbage.
    // Decode completely before writing so malformed input cannot leave partial ciphertext.
    let normalized: Vec<u8> = encoded_secret
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let decoded = BASE64
        .decode(normalized)
        .context("secret is not valid base64")?;
    tokio::fs::write(output_path, decoded)
        .await
        .with_context(|| format!("writing {}", output_path.display()))
}

pub(crate) fn encode_base64_single_line(input: &[u8]) -> String {
    BASE64.encode(input)
}

pub(crate) async fn write_stdin_and_wait(
    mut child: tokio::process::Child,
    input: &[u8],
) -> Result<std::process::Output> {
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

    async fn read_output(stream: Option<impl AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stream {
            stream.read_to_end(&mut bytes).await?;
        }
        Ok(bytes)
    }

    let mut stdin = child.stdin.take().context("opening child stdin")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Encryption can produce output before consuming the complete plaintext.
    // Drain both output pipes while writing, including waiting for stdin EOF.
    let result = tokio::try_join!(
        async move {
            stdin.write_all(input).await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        },
        read_output(stdout),
        read_output(stderr),
        child.wait(),
    );
    match result {
        Ok(((), stdout, stderr, status)) => Ok(std::process::Output {
            status,
            stdout,
            stderr,
        }),
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error).context("communicating with encryption process")
        }
    }
}

/// Bounded, zeroizing output for data-key unwraps; never echo backend stderr.
pub(crate) async fn read_key_output(mut child: tokio::process::Child) -> Result<String> {
    use tokio::io::AsyncReadExt;
    use zeroize::Zeroizing;
    async fn bounded(
        reader: impl tokio::io::AsyncRead + Unpin,
        limit: usize,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let mut bytes = Zeroizing::new(Vec::new());
        reader
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        anyhow::ensure!(bytes.len() <= limit, "hybrid unwrap output exceeds limit");
        Ok(bytes)
    }
    let stdout = child.stdout.take().context("opening unwrap output")?;
    let stderr = child.stderr.take();
    let result = tokio::try_join!(bounded(stdout, 96), async {
        if let Some(stderr) = stderr {
            bounded(stderr, 65536).await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    let bytes = match result {
        Ok((bytes, ())) => bytes,
        Err(error) => {
            let _ = child.kill().await;
            return Err(error);
        }
    };
    anyhow::ensure!(
        child.wait().await?.success(),
        "hybrid unwrap failed or was cancelled; no other backend was attempted"
    );
    Ok(std::str::from_utf8(&bytes)
        .context("invalid hybrid key text")?
        .to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn encryption_drains_both_pipes_while_writing_large_input() {
        let child = tokio::process::Command::new("sh")
            .args(["-c", "dd if=/dev/zero bs=16384 count=64 >&2; cat"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = vec![b'x'; 1024 * 1024];
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            write_stdin_and_wait(child, &input),
        )
        .await
        .expect("pipe deadlock")
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, input);
        assert!(output.stderr.starts_with(&vec![0; 1024 * 1024]));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn encryption_input_failure_kills_and_reaps_child() {
        let child = tokio::process::Command::new("sh")
            .args(["-c", "exec 0<&-; exec sleep 30"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            write_stdin_and_wait(child, &vec![b'x'; 1024 * 1024]),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        // Signal zero only probes existence; successful cleanup must reap the child.
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[tokio::test]
    async fn base64_standard_vectors_round_trip() {
        for (plain, encoded) in [
            (b"".as_slice(), ""),
            (b"f", "Zg=="),
            (b"ab", "YWI="),
            (b"foo", "Zm9v"),
            (b"\x00\xfb\xff", "APv/"),
        ] {
            assert_eq!(encode_base64_single_line(plain), encoded);
            let file = TempFile::new("shine-base64-test").await.unwrap();
            decode_base64_to_file(encoded, file.path()).await.unwrap();
            assert_eq!(tokio::fs::read(file.path()).await.unwrap(), plain);
        }
    }

    #[tokio::test]
    async fn base64_accepts_wrapped_ciphertext_and_preserves_binary_bytes() {
        let bytes: Vec<u8> = (0..=255).collect();
        let encoded = encode_base64_single_line(&bytes);
        assert!(!encoded.bytes().any(|byte| byte.is_ascii_whitespace()));
        let wrapped = encoded
            .as_bytes()
            .chunks(64)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n");
        let file = TempFile::new("shine-base64-test").await.unwrap();
        decode_base64_to_file(&format!(" \t{wrapped}\r\n"), file.path())
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(file.path()).await.unwrap(), bytes);
    }

    #[tokio::test]
    async fn base64_rejects_malformed_input_without_writing_or_echoing_it() {
        let file = TempFile::new("shine-base64-test").await.unwrap();
        tokio::fs::write(file.path(), b"unchanged").await.unwrap();
        for invalid in [
            "Zg",
            "Zg=",
            "Zg===",
            "====",
            "A",
            "Zh==",
            "Zg==AAAA",
            "Zg==!",
            "-_8=",
            "Zg==\u{a0}",
        ] {
            let err = decode_base64_to_file(invalid, file.path())
                .await
                .unwrap_err();
            assert_eq!(err.to_string(), "secret is not valid base64");
            assert!(!format!("{err:#}").contains(invalid));
            assert_eq!(tokio::fs::read(file.path()).await.unwrap(), b"unchanged");
        }
    }
}
