//! Local session agent: the server side of a `shine ssh` session's control
//! channel (the far end of the SSH `-R` forward). It serves file-transfer
//! requests and the optional on-demand secret broker. It no longer moves file
//! bytes itself — instead, on a `Transfer` request it spawns `rsync` (default)
//! or `scp` on the local machine, which opens its own ssh connection back to
//! the session host and moves the data directly, and relays the child's
//! stdout/stderr to the remote as `Log` frames plus a terminal `Done`
//! (ADR 0011).
//!
//! Transport: a Unix domain socket on macOS/Linux, or a loopback TCP socket on
//! Windows (verified via `scripts/spike-ssh-forward-windows.ps1` — Windows is
//! the local side only; the remote host is always Linux/macOS, so the
//! remote-side Unix socket and wrapped command in `ssh/mod.rs` never change).
//! [`LocalListener`] abstracts over the two; the per-connection protocol logic
//! below is transport-agnostic.
//!
//! Security: `remote_spec`/`local_spec` arrive from the remote over the wire
//! and are UNTRUSTED (the session token leaks via `ps eww` on the remote — see
//! docs/kb/lessons.md). The agent therefore executes rsync/scp with **argv
//! only, never a shell**; the remote path is only ever emitted as the single
//! token `<host>:<remote_spec>` after a `--` separator; and the ssh
//! reconnection options come solely from the local `SessionContext`, never the
//! wire. Secret requests additionally require an exact broker authorization;
//! the token alone never authorizes decryption.

mod connection;
mod transfer;
use connection::handle_connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use super::broker::BrokerSession;
use super::session_context::SessionContext;

/// Any duplex byte stream the agent can serve a connection over.
trait DuplexStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> DuplexStream for T {}

/// Tracks in-flight per-connection tasks so a caller can wait for them to
/// finish on their own (see [`drain_connection_tasks`]) instead of abandoning a
/// still-running transfer mid-copy when the accept loop is aborted or the
/// process is about to exit.
pub type ConnectionTasks = Arc<AsyncMutex<JoinSet<()>>>;

pub fn new_connection_tasks() -> ConnectionTasks {
    Arc::new(AsyncMutex::new(JoinSet::new()))
}

/// Waits, up to `grace_period`, for every currently tracked connection task to
/// finish. Once the local end of an `ssh` session's tunnel is gone, an
/// in-flight transfer's next socket read/write fails almost immediately and its
/// own error-path cleanup runs to completion here rather than being cut off by
/// `agent_handle.abort()` or process exit. Not a hard guarantee — a connection
/// that never notices the tunnel is gone is simply abandoned once the grace
/// period elapses, rather than blocking shutdown forever.
pub async fn drain_connection_tasks(tasks: &ConnectionTasks, grace_period: Duration) {
    let mut set = tasks.lock().await;
    let _ = tokio::time::timeout(grace_period, async {
        while set.join_next().await.is_some() {}
    })
    .await;
}

/// The local end of the session's transfer channel: a Unix socket (macOS/Linux)
/// or a loopback TCP socket (Windows). `tokio::net::UnixListener` only exists on
/// unix targets, hence the `Unix` variant is cfg-gated.
pub enum LocalListener {
    #[cfg(unix)]
    Unix(UnixListener),
    // Only constructed on Windows (see `ssh::bind_local_listener`); a
    // non-Windows build never builds that constructor, hence the allow.
    #[cfg_attr(not(windows), allow(dead_code))]
    Tcp(TcpListener),
}

impl LocalListener {
    /// Runs the accept loop until the listener is closed (session teardown).
    /// Each connection is handled on its own task tracked in `tasks` (see
    /// [`drain_connection_tasks`]); a failed connection is logged and does not
    /// bring down the agent or the session.
    pub async fn serve(
        self,
        token: String,
        context: Arc<SessionContext>,
        broker: Option<Arc<BrokerSession>>,
        tasks: ConnectionTasks,
    ) {
        match self {
            #[cfg(unix)]
            LocalListener::Unix(listener) => loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                spawn_connection(
                    stream,
                    token.clone(),
                    context.clone(),
                    broker.clone(),
                    &tasks,
                )
                .await;
            },
            LocalListener::Tcp(listener) => loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                spawn_connection(
                    stream,
                    token.clone(),
                    context.clone(),
                    broker.clone(),
                    &tasks,
                )
                .await;
            },
        }
    }
}

async fn spawn_connection(
    stream: impl DuplexStream,
    token: String,
    context: Arc<SessionContext>,
    broker: Option<Arc<BrokerSession>>,
    tasks: &ConnectionTasks,
) {
    tasks.lock().await.spawn(async move {
        if let Err(error) = handle_connection(stream, &token, &context, broker.as_deref()).await {
            eprintln!(
                "shine ssh: transfer agent connection error: {}",
                crate::output::escape_controls(&format!("{error:#}"))
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connection_error_notice_encodes_peer_data_and_drains_the_task() {
        const CHILD: &str = "SHINE_TEST_SSH_CONNECTION_NOTICE";
        if std::env::var_os(CHILD).is_some() {
            use tokio::io::AsyncReadExt;
            let (mut peer, stream) = tokio::io::duplex(2048);
            let tasks = new_connection_tasks();
            let context = Arc::new(SessionContext {
                host: "never-connect.invalid".into(),
                ssh_options: vec![],
                local_dir: std::env::temp_dir(),
                control_path: None,
            });
            spawn_connection(stream, "secret-session-token".into(), context, None, &tasks).await;
            crate::ssh::protocol::write_message(
                &mut peer,
                &serde_json::json!({"type":"unknown\nFORGED\r\t\u{1b}"}),
            )
            .await
            .unwrap();
            drain_connection_tasks(&tasks, Duration::from_secs(2)).await;
            assert!(tasks.lock().await.is_empty());
            assert_eq!(peer.read(&mut [0_u8; 1]).await.unwrap(), 0);
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ssh::agent::tests::connection_error_notice_encodes_peer_data_and_drains_the_task",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert!(!stderr.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
        assert!(stderr.contains(r"unknown\nFORGED\r\t\u{1b}"), "{stderr:?}");
        assert!(stderr.contains("failed to parse protocol message"));
        assert!(!stderr.contains("secret-session-token"));
    }
}
