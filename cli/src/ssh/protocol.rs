//! Wire protocol spoken over the forwarded Unix socket between remote
//! `shine local` / `shine env run --secret-broker` clients and the local
//! `shine ssh` session agent. See docs/ssh-local-transfer-prd.md section 9 and
//! docs/ssh-secret-broker-prd.md.
//!
//! The socket carries a **control + log-relay** channel, not file bytes: the
//! remote sends one `Transfer` request, the local agent runs `rsync`/`scp`
//! (which moves the bytes over its own ssh connection), and the child's
//! stdout/stderr are relayed back as `Log` frames followed by a terminal
//! `Done`. See ADR 0011.
//!
//! Framing: every message is JSON, prefixed by a `u32` little-endian byte
//! length. There is no separate raw-byte stream — `Log` chunks travel inside
//! ordinary control frames, so the frame-size cap bounds them too.

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bumped whenever `ClientMessage`/`ServerMessage` change in an
/// incompatible way. Local agent and remote client must match exactly.
///
/// v2: switched from custom byte-streaming (`PutFile`/`GetFile` + raw bodies)
/// to a control channel driving a local `rsync`/`scp` (ADR 0011).
/// v3: added session-scoped direct/workspace secret broker requests.
pub const PROTOCOL_VERSION: u32 = 3;

/// There is no separate raw file-body stream. This cap bounds both broker
/// workspace/source snapshots and metadata/log frames against a hostile peer;
/// field-level broker validation applies tighter limits after deserialization.
const MAX_CONTROL_FRAME_BYTES: u32 = 2 * 1024 * 1024;

/// Which way a transfer moves relative to the machine that owns each path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Remote → local (`shine local download`): the remote spec is the source.
    Download,
    /// Local → remote (`shine local upload`): the remote spec is the destination.
    Upload,
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Hello {
        protocol_version: u32,
    },
    /// A single transfer request. The local agent validates `token`, resolves
    /// the local side against the session directory, and spawns `rsync`/`scp`.
    Transfer {
        token: String,
        direction: Direction,
        /// The remote-owned spec, already absolutized against the remote cwd
        /// by `remote_client` with glob metacharacters preserved. UNTRUSTED:
        /// the agent only ever emits it as the single argv token
        /// `<host>:<remote_spec>` after a `--` separator, never as a bare
        /// option and never near `-e`/`-o`/`--rsh`/`--rsync-path`.
        remote_spec: String,
        /// The local-owned path/hint (download destination, upload source) as
        /// typed by the user. UNTRUSTED: resolved by the agent against the
        /// session directory with tilde-only expansion; an upload source is
        /// glob-expanded locally.
        local_spec: Option<String>,
        /// Overwrite semantics: for rsync, `false` maps to `--ignore-existing`.
        force: bool,
        /// Preview only — no bytes are moved.
        dry_run: bool,
        /// Force `scp` instead of the default `rsync`.
        use_scp: bool,
    },
    /// Report session/connection info without transferring anything.
    Status {
        token: String,
    },
    /// Request explicitly session-authorized local config secrets. This path
    /// always requires local interactive confirmation.
    DirectSecret {
        token: String,
        specs: Vec<String>,
        argv: Vec<String>,
        nonce: String,
    },
    /// Request secrets from sealed workspace sources after exact local policy
    /// matching and local recomputation of every digest.
    WorkspaceSecret {
        token: String,
        snapshot: crate::env::broker::WorkspaceSnapshot,
        argv: Vec<String>,
        nonce: String,
    },
    /// Describe a workspace request for local inspect/trusted-enrollment mode.
    /// This request never decrypts or returns secret values.
    DescribeWorkspace {
        token: String,
        snapshot: crate::env::broker::WorkspaceSnapshot,
        release: Vec<String>,
        argv: Vec<String>,
        nonce: String,
    },
}

/// The external tool the local agent ran for a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Rsync,
    Scp,
}

/// Which of the child's output streams a `Log` chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    HelloAck {
        protocol_version: u32,
    },
    /// Sent once before the first `Log` frame: which tool ran and whether we
    /// auto-fell-back from rsync to scp (with a human note to print).
    Starting {
        tool: Tool,
        fell_back: bool,
        note: Option<String>,
    },
    /// A raw fragment of the child's stdout/stderr, forwarded verbatim (it may
    /// contain `\r` and no trailing newline, preserving rsync/scp progress
    /// redraws — the remote writes it without adding a newline).
    Log {
        stream: LogStream,
        chunk: String,
    },
    /// Terminal frame of a transfer: the child's exit code, propagated to the
    /// remote `shine local` process's own exit status.
    Done {
        code: i32,
    },
    /// Response to a `Status` request.
    StatusResponse {
        session_local_dir: String,
        host: String,
    },
    SecretResponse {
        values: BTreeMap<String, String>,
    },
    DescriptionResponse {
        summary: String,
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
    async fn round_trips_a_transfer_request() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &ClientMessage::Transfer {
                token: "tok".to_string(),
                direction: Direction::Download,
                remote_spec: "/abs/logs/*.log".to_string(),
                local_spec: Some("dest".to_string()),
                force: true,
                dry_run: false,
                use_scp: false,
            },
        )
        .await
        .unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: ClientMessage = read_message(&mut cursor).await.unwrap();
        match decoded {
            ClientMessage::Transfer {
                direction,
                remote_spec,
                force,
                ..
            } => {
                assert_eq!(direction, Direction::Download);
                assert_eq!(remote_spec, "/abs/logs/*.log");
                assert!(force);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn round_trips_a_direct_secret_request_without_exposing_values() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &ClientMessage::DirectSecret {
                token: "session-token".into(),
                specs: vec!["API_TOKEN".into()],
                argv: vec!["bun".into(), "run".into(), "build".into()],
                nonce: "12345678-1234-1234-1234-123456789abc".into(),
            },
        )
        .await
        .unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let decoded: ClientMessage = read_message(&mut cursor).await.unwrap();
        assert!(matches!(
            decoded,
            ClientMessage::DirectSecret { specs, argv, .. }
                if specs == ["API_TOKEN"] && argv == ["bun", "run", "build"]
        ));
    }

    #[tokio::test]
    async fn round_trips_server_log_and_done() {
        for message in [
            ServerMessage::Starting {
                tool: Tool::Scp,
                fell_back: true,
                note: Some("rsync not found; using scp".to_string()),
            },
            ServerMessage::Log {
                stream: LogStream::Stderr,
                chunk: "progress\r".to_string(),
            },
            ServerMessage::Done { code: 23 },
            ServerMessage::StatusResponse {
                session_local_dir: "/home/u".to_string(),
                host: "dev".to_string(),
            },
        ] {
            let mut buf = Vec::new();
            write_message(&mut buf, &message).await.unwrap();
            let mut cursor = std::io::Cursor::new(buf);
            let _decoded: ServerMessage = read_message(&mut cursor).await.unwrap();
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
}
