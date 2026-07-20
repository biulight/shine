//! In-process integration tests for the transfer control protocol: a real
//! `tokio::net::UnixListener`/`UnixStream` pair, the real `agent` server, and
//! hand-driven protocol messages standing in for `remote_client`. The agent
//! spawns `rsync`/`scp` via `resolve_tool_binary`, which honors the
//! `SHINE_TEST_RSYNC`/`SHINE_TEST_SCP` overrides — pointed here at a stub
//! script — so no real rsync/scp or ssh host is involved.
#![allow(clippy::await_holding_lock)] // env_lock() is held across awaits, per test_support convention

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::agent;
use super::protocol::{self, ClientMessage, Direction, PROTOCOL_VERSION, ServerMessage};
use super::session_context::SessionContext;
use crate::test_support;

struct TempDir(PathBuf);

impl TempDir {
    /// Uses `/tmp` directly rather than `std::env::temp_dir()`: on macOS the
    /// latter resolves to a long `/var/folders/...` path that, combined with a
    /// socket file name, can exceed `sockaddr_un`'s ~104-byte `SUN_LEN` limit.
    fn new() -> Self {
        let path = PathBuf::from("/tmp").join(format!("shine-ssh-it-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes an executable stub that stands in for rsync: it exits 0 on
/// `--version` (so the availability probe passes) and otherwise prints a known
/// line to stdout and stderr and exits with `exit_code`.
fn write_stub(dir: &Path, exit_code: i32) -> PathBuf {
    let path = dir.join("tool-stub.sh");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo 'stub 1.0'; exit 0; fi\n\
         printf 'STUB-OUT\\n'\n\
         printf 'STUB-ERR\\n' 1>&2\n\
         exit {exit_code}\n"
    );
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn context_in(local_dir: &Path) -> SessionContext {
    // control_path=None keeps the agent from shelling out to a real `ssh` for
    // the remote-rsync probe; ssh_options empty means rsync needs no `-e`.
    SessionContext {
        host: "dev".to_string(),
        ssh_options: vec![],
        local_dir: local_dir.to_path_buf(),
        control_path: None,
    }
}

async fn start_agent(token: &str, context: SessionContext) -> PathBuf {
    let sock_dir = TempDir::new();
    let sock_path = sock_dir.path().join("local.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    let tasks = agent::new_connection_tasks();
    tokio::spawn(agent::LocalListener::Unix(listener).serve(
        token.to_string(),
        Arc::new(context),
        tasks,
    ));
    // Keep the socket file alive for the test's lifetime; each test uses a
    // fresh uuid-named path so leaked directories never collide.
    std::mem::forget(sock_dir);
    sock_path
}

async fn handshake(sock_path: &Path) -> tokio::net::UnixStream {
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

/// Reads frames until `Done`/`Error`, concatenating all `Log` chunks.
async fn drain(stream: &mut tokio::net::UnixStream) -> (String, Option<i32>) {
    let mut logs = String::new();
    loop {
        match protocol::read_message(stream).await.unwrap() {
            ServerMessage::Starting { .. } => {}
            ServerMessage::Log { chunk, .. } => logs.push_str(&chunk),
            ServerMessage::Done { code } => return (logs, Some(code)),
            ServerMessage::Error { message } => {
                logs.push_str(&message);
                return (logs, None);
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}

#[tokio::test]
async fn download_relays_child_output_and_zero_exit_code() {
    let _env = test_support::env_lock();
    let work = TempDir::new();
    let stub = write_stub(work.path(), 0);
    unsafe { std::env::set_var("SHINE_TEST_RSYNC", &stub) };

    let sock = start_agent("tok", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: "tok".to_string(),
            direction: Direction::Download,
            remote_spec: "/abs/file".to_string(),
            local_spec: None,
            force: false,
            dry_run: false,
            use_scp: false,
        },
    )
    .await
    .unwrap();

    let (logs, code) = drain(&mut stream).await;
    unsafe { std::env::remove_var("SHINE_TEST_RSYNC") };
    assert!(logs.contains("STUB-OUT"), "stdout not relayed: {logs:?}");
    assert!(logs.contains("STUB-ERR"), "stderr not relayed: {logs:?}");
    assert_eq!(code, Some(0));
}

#[tokio::test]
async fn nonzero_child_exit_code_is_reported() {
    let _env = test_support::env_lock();
    let work = TempDir::new();
    let stub = write_stub(work.path(), 7);
    unsafe { std::env::set_var("SHINE_TEST_RSYNC", &stub) };

    let sock = start_agent("tok", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: "tok".to_string(),
            direction: Direction::Download,
            remote_spec: "/abs/file".to_string(),
            local_spec: None,
            force: false,
            dry_run: false,
            use_scp: false,
        },
    )
    .await
    .unwrap();

    let (_logs, code) = drain(&mut stream).await;
    unsafe { std::env::remove_var("SHINE_TEST_RSYNC") };
    assert_eq!(code, Some(7));
}

#[tokio::test]
async fn invalid_token_is_rejected_without_running_anything() {
    let work = TempDir::new();
    let sock = start_agent("correct", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: "wrong".to_string(),
            direction: Direction::Download,
            remote_spec: "/abs/file".to_string(),
            local_spec: None,
            force: false,
            dry_run: false,
            use_scp: false,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
}

#[tokio::test]
async fn scp_dry_run_synthesizes_a_preview_without_spawning() {
    let work = TempDir::new();
    let sock = start_agent("tok", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: "tok".to_string(),
            direction: Direction::Download,
            remote_spec: "/abs/file".to_string(),
            local_spec: None,
            force: false,
            dry_run: true,
            use_scp: true,
        },
    )
    .await
    .unwrap();

    let (logs, code) = drain(&mut stream).await;
    assert!(logs.contains("Would copy"), "no preview line: {logs:?}");
    assert_eq!(code, Some(0));
}

#[tokio::test]
async fn upload_glob_with_no_matches_is_an_error() {
    let _env = test_support::env_lock();
    let work = TempDir::new();
    let stub = write_stub(work.path(), 0);
    unsafe { std::env::set_var("SHINE_TEST_RSYNC", &stub) };

    let sock = start_agent("tok", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: "tok".to_string(),
            direction: Direction::Upload,
            remote_spec: "/remote/dir".to_string(),
            local_spec: Some("*.nonexistent".to_string()),
            force: false,
            dry_run: false,
            use_scp: false,
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    unsafe { std::env::remove_var("SHINE_TEST_RSYNC") };
    assert!(matches!(response, ServerMessage::Error { .. }));
}

#[tokio::test]
async fn status_reports_host_and_local_dir() {
    let work = TempDir::new();
    let sock = start_agent("tok", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Status {
            token: "tok".to_string(),
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    match response {
        ServerMessage::StatusResponse {
            session_local_dir,
            host,
        } => {
            assert_eq!(session_local_dir, work.path().display().to_string());
            assert_eq!(host, "dev");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn status_with_an_invalid_token_is_rejected() {
    let work = TempDir::new();
    let sock = start_agent("correct", context_in(work.path())).await;
    let mut stream = handshake(&sock).await;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Status {
            token: "wrong".to_string(),
        },
    )
    .await
    .unwrap();

    let response: ServerMessage = protocol::read_message(&mut stream).await.unwrap();
    assert!(matches!(response, ServerMessage::Error { .. }));
}
