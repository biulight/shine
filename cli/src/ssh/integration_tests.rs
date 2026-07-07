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
    tokio::spawn(agent::LocalListener::Unix(listener).serve(token.to_string(), session_local_dir));
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
            is_dir: false,
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
            is_dir: false,
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
            is_dir: false,
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
            is_dir: false,
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
async fn download_round_trip_handles_filenames_with_spaces_and_non_ascii_characters() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let filename = "has space and 中文.txt";
    let content = b"tricky filename content";
    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: filename.to_string(),
            is_dir: false,
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
        ServerMessage::PutAck { resolved_path, .. } => {
            assert!(resolved_path.ends_with(filename));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let written = std::fs::read(session_local_dir.path().join(filename)).unwrap();
    assert_eq!(written, content);
}

#[tokio::test]
async fn upload_round_trip_handles_filenames_with_spaces_and_non_ascii_characters() {
    let session_local_dir = TempDir::new();
    let filename = "notes with spaces 和汉字.txt";
    let content = b"tricky filename content to send";
    std::fs::write(session_local_dir.path().join(filename), content).unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::GetFile {
            token: token.to_string(),
            source_hint: filename.to_string(),
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let header: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    let (resolved_path, size) = match header {
        ServerMessage::GetHeader {
            resolved_path,
            size,
            ..
        } => (resolved_path, size),
        other => panic!("unexpected response: {other:?}"),
    };
    assert!(resolved_path.ends_with(filename));

    let mut received = Vec::new();
    protocol::copy_exact(&mut stream, &mut received, size)
        .await
        .unwrap();
    assert_eq!(received, content);
}

#[tokio::test]
async fn download_directory_round_trip_preserves_filenames_with_spaces_and_non_ascii_characters() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let tricky_name = "nested dir 目录/文件 with space.txt";
    let source = TempDir::new();
    std::fs::create_dir_all(source.path().join("nested dir 目录")).unwrap();
    std::fs::write(source.path().join(tricky_name), b"tricky nested content").unwrap();

    let (tar_path, tar_size) =
        super::dir_transfer::build_tar_to_temp_file(source.path().to_path_buf())
            .await
            .unwrap();

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "mydir".to_string(),
            is_dir: true,
            size: tar_size,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));
    let mut tar_file = tokio::fs::File::open(&tar_path).await.unwrap();
    protocol::copy_exact(&mut tar_file, &mut stream, tar_size)
        .await
        .unwrap();
    let _ = tokio::fs::remove_file(&tar_path).await;
    let ack: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(ack, ServerMessage::PutAck { .. }));

    let dest_dir = session_local_dir.path().join("mydir");
    assert_eq!(
        std::fs::read(dest_dir.join(tricky_name)).unwrap(),
        b"tricky nested content"
    );
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
            is_dir: false,
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

#[tokio::test]
async fn download_directory_round_trip_preserves_nested_files_and_safe_symlinks() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let source = TempDir::new();
    std::fs::create_dir_all(source.path().join("nested")).unwrap();
    std::fs::write(source.path().join("nested/file.txt"), b"hello").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("nested/file.txt", source.path().join("link.txt")).unwrap();

    let (tar_path, tar_size) =
        super::dir_transfer::build_tar_to_temp_file(source.path().to_path_buf())
            .await
            .unwrap();

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "mydir".to_string(),
            is_dir: true,
            size: tar_size,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));

    let mut tar_file = tokio::fs::File::open(&tar_path).await.unwrap();
    protocol::copy_exact(&mut tar_file, &mut stream, tar_size)
        .await
        .unwrap();
    let _ = tokio::fs::remove_file(&tar_path).await;

    let ack: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(ack, ServerMessage::PutAck { .. }));

    let dest_dir = session_local_dir.path().join("mydir");
    assert_eq!(
        std::fs::read(dest_dir.join("nested/file.txt")).unwrap(),
        b"hello"
    );
    #[cfg(unix)]
    {
        let link_target = std::fs::read_link(dest_dir.join("link.txt")).unwrap();
        assert_eq!(link_target, std::path::Path::new("nested/file.txt"));
    }
}

#[tokio::test]
async fn download_directory_without_force_rejects_an_existing_destination_directory() {
    let session_local_dir = TempDir::new();
    std::fs::create_dir_all(session_local_dir.path().join("mydir")).unwrap();
    std::fs::write(session_local_dir.path().join("mydir/keep.txt"), b"keep").unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let source = TempDir::new();
    std::fs::write(source.path().join("new.txt"), b"new").unwrap();
    let (tar_path, tar_size) =
        super::dir_transfer::build_tar_to_temp_file(source.path().to_path_buf())
            .await
            .unwrap();

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "mydir".to_string(),
            is_dir: true,
            size: tar_size,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let _ = tokio::fs::remove_file(&tar_path).await;

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
    // The unrelated existing file must survive the rejected transfer.
    assert_eq!(
        std::fs::read(session_local_dir.path().join("mydir/keep.txt")).unwrap(),
        b"keep"
    );
}

#[tokio::test]
async fn download_directory_with_force_merges_into_an_existing_destination_directory() {
    let session_local_dir = TempDir::new();
    std::fs::create_dir_all(session_local_dir.path().join("mydir")).unwrap();
    std::fs::write(session_local_dir.path().join("mydir/keep.txt"), b"keep").unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let source = TempDir::new();
    std::fs::write(source.path().join("new.txt"), b"new").unwrap();
    let (tar_path, tar_size) =
        super::dir_transfer::build_tar_to_temp_file(source.path().to_path_buf())
            .await
            .unwrap();

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "mydir".to_string(),
            is_dir: true,
            size: tar_size,
            force: true,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));
    let mut tar_file = tokio::fs::File::open(&tar_path).await.unwrap();
    protocol::copy_exact(&mut tar_file, &mut stream, tar_size)
        .await
        .unwrap();
    let _ = tokio::fs::remove_file(&tar_path).await;
    let ack: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(ack, ServerMessage::PutAck { .. }));

    let dest_dir = session_local_dir.path().join("mydir");
    assert_eq!(std::fs::read(dest_dir.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(std::fs::read(dest_dir.join("new.txt")).unwrap(), b"new");
}

#[cfg(unix)]
#[tokio::test]
async fn download_directory_rejects_an_escaping_symlink_via_protocol_error() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let source = TempDir::new();
    std::fs::write(source.path().join("real.txt"), b"data").unwrap();
    std::os::unix::fs::symlink("../../../etc/passwd", source.path().join("evil")).unwrap();
    let (tar_path, tar_size) =
        super::dir_transfer::build_tar_to_temp_file(source.path().to_path_buf())
            .await
            .unwrap();

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: token.to_string(),
            dest_hint: None,
            filename: "mydir".to_string(),
            is_dir: true,
            size: tar_size,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let proceed: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));
    let mut tar_file = tokio::fs::File::open(&tar_path).await.unwrap();
    protocol::copy_exact(&mut tar_file, &mut stream, tar_size)
        .await
        .unwrap();
    let _ = tokio::fs::remove_file(&tar_path).await;

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
    // The malicious entry itself must never be created, regardless of
    // whether earlier safe entries in the same archive were extracted
    // before the unsafe one was reached.
    assert!(!session_local_dir.path().join("mydir/evil").exists());
}

#[tokio::test]
async fn upload_directory_round_trip_reads_local_directory_content() {
    let session_local_dir = TempDir::new();
    std::fs::create_dir_all(session_local_dir.path().join("mydir/nested")).unwrap();
    std::fs::write(
        session_local_dir.path().join("mydir/nested/file.txt"),
        b"payload",
    )
    .unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::GetFile {
            token: token.to_string(),
            source_hint: "mydir".to_string(),
            dry_run: false,
        },
    )
    .await
    .unwrap();

    let header: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    let (is_dir, size) = match header {
        ServerMessage::GetHeader { is_dir, size, .. } => (is_dir, size),
        other => panic!("unexpected response: {other:?}"),
    };
    assert!(is_dir);

    let dest = TempDir::new();
    let temp_tar = dest.path().join("received.tar");
    let mut temp_file = tokio::fs::File::create(&temp_tar).await.unwrap();
    protocol::copy_exact(&mut stream, &mut temp_file, size)
        .await
        .unwrap();
    drop(temp_file);

    let extract_dest = dest.path().join("extracted");
    super::dir_transfer::extract_tar_from_file(temp_tar, extract_dest.clone())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(extract_dest.join("nested/file.txt")).unwrap(),
        b"payload"
    );
}

#[tokio::test]
async fn upload_directory_dry_run_does_not_send_a_tar() {
    let session_local_dir = TempDir::new();
    std::fs::create_dir_all(session_local_dir.path().join("mydir")).unwrap();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::GetFile {
            token: token.to_string(),
            source_hint: "mydir".to_string(),
            dry_run: true,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(
        response,
        ServerMessage::Preview {
            is_dir: true,
            size: None,
            ..
        }
    ));
}

#[tokio::test]
async fn status_reports_the_session_local_dir() {
    let session_local_dir = TempDir::new();
    let token = "test-token";
    let sock_path = start_agent(token, session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Status {
            token: token.to_string(),
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    match response {
        ServerMessage::StatusResponse {
            session_local_dir: reported,
        } => {
            assert_eq!(reported, session_local_dir.path().display().to_string());
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_sessions_do_not_share_tokens_or_default_directories() {
    let session_a_dir = TempDir::new();
    let session_b_dir = TempDir::new();
    let token_a = "token-a";
    let token_b = "token-b";
    let sock_a = start_agent(token_a, session_a_dir.path().to_path_buf()).await;
    let sock_b = start_agent(token_b, session_b_dir.path().to_path_buf()).await;

    // Session A's token must not be honored against session B's socket.
    let mut cross_stream = handshake(&sock_b).await;
    protocol::write_message(
        &mut cross_stream,
        &ClientMessage::PutFile {
            token: token_a.to_string(),
            dest_hint: None,
            filename: "leak.log".to_string(),
            is_dir: false,
            size: 3,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let response: ServerMessage = protocol::read_message(&mut cross_stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
    assert!(!session_b_dir.path().join("leak.log").exists());

    // A request using session A's own token against its own socket must
    // land only in session A's directory, never session B's.
    let content_a = b"data for session a";
    let mut stream_a = handshake(&sock_a).await;
    protocol::write_message(
        &mut stream_a,
        &ClientMessage::PutFile {
            token: token_a.to_string(),
            dest_hint: None,
            filename: "result.log".to_string(),
            is_dir: false,
            size: content_a.len() as u64,
            force: false,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    let proceed: ServerMessage = protocol::read_message(&mut stream_a).await.unwrap();
    assert!(matches!(proceed, ServerMessage::Proceed));
    let mut cursor = std::io::Cursor::new(content_a.to_vec());
    protocol::copy_exact(&mut cursor, &mut stream_a, content_a.len() as u64)
        .await
        .unwrap();
    let _: ServerMessage = protocol::read_message(&mut stream_a).await.unwrap();

    assert_eq!(
        std::fs::read(session_a_dir.path().join("result.log")).unwrap(),
        content_a
    );
    assert!(!session_b_dir.path().join("result.log").exists());
}

#[tokio::test]
async fn status_with_an_invalid_token_is_rejected() {
    let session_local_dir = TempDir::new();
    let sock_path = start_agent("correct-token", session_local_dir.path().to_path_buf()).await;

    let mut stream = handshake(&sock_path).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Status {
            token: "wrong-token".to_string(),
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
}
