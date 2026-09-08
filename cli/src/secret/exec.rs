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

#[cfg(test)]
mod tests {
    use super::*;

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
