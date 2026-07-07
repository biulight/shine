//! Wire protocol spoken over the forwarded Unix socket between the remote
//! `shine local download/upload` process (client) and the local `shine ssh`
//! transfer agent (server). See docs/ssh-local-transfer-prd.md section 9.
//!
//! Framing: every control message is JSON, prefixed by a `u32` little-endian
//! byte length. File content that follows a `PutFile`/`GetHeader` message is
//! raw bytes of the declared `size`, with no additional framing, streamed in
//! fixed-size chunks so memory use stays bounded regardless of file size.

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bumped whenever `ClientMessage`/`ServerMessage` change in an
/// incompatible way. Local agent and remote client must match exactly.
pub const PROTOCOL_VERSION: u32 = 1;

/// Control frames are metadata only; file bodies are streamed separately.
/// This bounds accidental memory blowup from a malformed/hostile peer.
const MAX_CONTROL_FRAME_BYTES: u32 = 1024 * 1024;

const COPY_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Hello {
        protocol_version: u32,
    },
    /// "shine local download": remote already read its own source file and
    /// is pushing the bytes to be written on the local machine.
    PutFile {
        token: String,
        /// Raw local-destination argument as typed by the user, resolved
        /// by the local agent against the session's local directory.
        dest_hint: Option<String>,
        /// Basename of the remote source path, used when `dest_hint` is
        /// absent or resolves to an existing directory.
        filename: String,
        /// If true, `size` bytes are an uncompressed tar stream to be
        /// extracted into the resolved destination directory rather than
        /// written verbatim as a single file.
        is_dir: bool,
        size: u64,
        force: bool,
        /// If true, only resolve the destination and report what would
        /// happen; no file bytes are sent and none should be read back.
        dry_run: bool,
    },
    /// "shine local upload": remote wants to read a file that lives on the
    /// local machine so it can write it to the remote destination itself.
    GetFile {
        token: String,
        /// Raw local-source argument as typed by the user, resolved by the
        /// local agent against the session's local directory.
        source_hint: String,
        /// If true, only resolve/stat the source and report it; no file
        /// bytes are sent.
        dry_run: bool,
    },
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    HelloAck {
        protocol_version: u32,
    },
    /// Sent after validating a `PutFile` header (token, overwrite policy)
    /// and before the client starts streaming bytes.
    Proceed,
    PutAck {
        resolved_path: String,
        bytes_written: u64,
    },
    GetHeader {
        resolved_path: String,
        /// If true, `size` bytes are an uncompressed tar stream of the
        /// resolved local directory rather than a single file's content.
        is_dir: bool,
        size: u64,
    },
    /// Response to a `dry_run` request: no bytes were or will be sent.
    Preview {
        resolved_path: String,
        is_dir: bool,
        size: Option<u64>,
        would_overwrite: bool,
    },
    Error {
        message: String,
    },
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(message).context("failed to serialize protocol message")?;
    let len = u32::try_from(payload.len()).context("protocol message too large")?;
    writer
        .write_all(&len.to_le_bytes())
        .await
        .context("failed to write protocol frame length")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write protocol frame body")?;
    writer
        .flush()
        .await
        .context("failed to flush protocol frame")?;
    Ok(())
}

pub async fn read_message<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_bytes = [0u8; 4];
    reader
        .read_exact(&mut len_bytes)
        .await
        .context("failed to read protocol frame length")?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_CONTROL_FRAME_BYTES {
        bail!("protocol control frame of {len} bytes exceeds the maximum allowed size");
    }
    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read protocol frame body")?;
    serde_json::from_slice(&payload).context("failed to parse protocol message")
}

/// Streams exactly `len` bytes from `reader` to `writer` in bounded-size
/// chunks, so transferring a large file never buffers it all in memory.
pub async fn copy_exact<R, W>(reader: &mut R, writer: &mut W, len: u64) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut remaining = len;
    let mut buf = vec![0u8; COPY_CHUNK_SIZE];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        reader
            .read_exact(&mut buf[..want])
            .await
            .context("failed to read file content from peer")?;
        writer
            .write_all(&buf[..want])
            .await
            .context("failed to write file content")?;
        remaining -= want as u64;
    }
    writer
        .flush()
        .await
        .context("failed to flush file content")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_a_control_message() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
            },
        )
        .await
        .unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: ClientMessage = read_message(&mut cursor).await.unwrap();
        match decoded {
            ClientMessage::Hello { protocol_version } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_oversized_control_frame() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_CONTROL_FRAME_BYTES + 1).to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let result: Result<ClientMessage> = read_message(&mut cursor).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn copy_exact_streams_the_declared_length() {
        let source = b"hello world, this is streamed content".to_vec();
        let mut reader = std::io::Cursor::new(source.clone());
        let mut dest = Vec::new();
        copy_exact(&mut reader, &mut dest, source.len() as u64)
            .await
            .unwrap();
        assert_eq!(dest, source);
    }
}
