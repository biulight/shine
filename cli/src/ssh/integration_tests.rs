//! In-process integration tests for the transfer protocol: a real
//! `tokio::net::UnixListener`/`UnixStream` pair, the real `agent` server,
//! and hand-driven protocol messages standing in for `remote_client` — no
//! real `ssh` subprocess involved. Verifies download/upload round trips,
//! overwrite protection, `--force`, and `--dry-run` end to end.

use std::path::PathBuf;

use super::agent;
use super::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};

struct TempDir(PathBuf);

impl TempDir {
    /// Uses `/tmp` directly rather than `std::env::temp_dir()`: on macOS the
    /// latter resolves to a long `/var/folders/...` path that, combined with
    /// a socket file name, can exceed `sockaddr_un`'s ~104-byte `SUN_LEN`
    /// limit. Production code has the same constraint (see `remote_sock` in
    /// `ssh/mod.rs`, which also anchors under `/tmp`).
    fn new() -> Self {
        let path =
            std::path::PathBuf::from("/tmp").join(format!("shine-ssh-it-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn start_agent(token: &str, session_local_dir: PathBuf) -> PathBuf {
    let sock_dir = TempDir::new();
    let sock_path = sock_dir.path().join("local.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(agent::serve(listener, token.to_string(), session_local_dir));
    // Keep the temp dir alive for the socket file's lifetime by leaking it;
    // each test uses a fresh uuid-named path so leaked directories never
    // collide, and the OS temp dir is cleaned up independently.
    std::mem::forget(sock_dir);
    sock_path
}

async fn handshake(sock_path: &std::path::Path) -> tokio::net::UnixStream {
    let mut stream = tokio::net::UnixStream::connect(sock_path).await.unwrap();
    protocol::write_message(
        &mut stream,
        &ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await
    .unwrap();
    let ack: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(ack, ServerMessage::HelloAck { .. }));
    stream
}

#[tokio::test]
async fn download_round_trip_writes_file_with_matching_content() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let content = b"hello from the remote host";
    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "result.log".to_string(),
            size: content.len() as u64,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));

    let mut cursor = std::io::Cursor::new(content.to_vec());
    protocol::copy_exact(&mut cursor, &mut stream, content.len() as u64)
        .await
        .unwrap();

    let ack: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    match ack {
        ServerMessage::PutAck { bytes_written, .. } => {
            assert_eq!(bytes_written, content.len() as u64);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let written = std::fs::read(session_local_dir.path().join("result.log")).unwrap();
    assert_eq!(written, content);
}

#[tokio::test]
async fn download_without_force_rejects_an_existing_destination() {
    let session_local_dir = TempDir::new();
    std::fs::write(session_local_dir.path().join("result.log"), b"old").unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "result.log".to_string(),
            size: 3,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
    // The rejected destination must be left untouched.
    assert_eq!(
        std::fs::read(session_local_dir.path().join("result.log")).unwrap(),
        b"old"
    );
}

#[tokio::test]
async fn download_with_force_overwrites_an_existing_destination() {
    let session_local_dir = TempDir::new();
    std::fs::write(session_local_dir.path().join("result.log"), b"old-content").unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let new_content = b"new";
    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "result.log".to_string(),
            size: new_content.len() as u64,
            force: true,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));
    let mut cursor = std::io::Cursor::new(new_content.to_vec());
    protocol::copy_exact(&mut cursor, &mut stream, new_content.len() as u64)
        .await
        .unwrap();
    let _: ServerMessage = protocol::read_message(&mut stream).await.unwrap();

    assert_eq!(
        std::fs::read(session_local_dir.path().join("result.log")).unwrap(),
        new_content
    );
}

#[tokio::test]
async fn upload_round_trip_reads_local_file_content() {
    let session_local_dir = TempDir::new();
    let content = b"notes to send to the remote";
    std::fs::write(session_local_dir.path().join("notes.txt"), content).unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::GetFile {
            token: token.to_string(),
            source_hint: "notes.txt".to_string(),
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let header: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    let size = match header {
        ServerMessage::GetHeader { size, .. } => size,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(size, content.len() as u64);

    let mut received = Vec::new();
    protocol::copy_exact(&mut stream, &mut received, size)
        .await
        .unwrap();
    assert_eq!(received, content);
}

#[tokio::test]
async fn dry_run_download_does_not_write_a_file() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "preview.log".to_string(),
            size: 4,
            force: false,
            dry_run: true,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(
        response,
        ServerMessage::Preview {
            would_overwrite: false,
            ..
        }
    ));
    assert!(!session_local_dir.path().join("preview.log").exists());
}

#[tokio::test]
async fn invalid_token_is_rejected() {
    let session_local_dir = TempDir::new();
    let sock_path = start_agent("correct-token", session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: "wrong-token".to_string(),
            dest_hint: None,
            filename: "result.log".to_string(),
            size: 3,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
}
