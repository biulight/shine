//! Local transfer agent: the server side of a `shine ssh` session's
//! transfer channel (the far end of the SSH `-R` forward), serving
//! `PutFile` (download) / `GetFile` (upload) requests from the remote
//! `shine local` client. Directories are handled by staging/extracting a
//! tar archive (see `dir_transfer.rs`).
//!
//! Transport: a Unix domain socket on macOS/Linux, or a loopback TCP
//! socket on Windows (verified via `scripts/spike-ssh-forward-windows.ps1`
//! — Windows is the local side only; the remote host is always
//! Linux/macOS, so the remote-side Unix socket and wrapped command in
//! `ssh/mod.rs` never change). [`LocalListener`] abstracts over the two;
//! the per-connection protocol logic below is transport-agnostic.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use super::dir_transfer;
use super::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use crate::home;

/// Any duplex byte stream the agent can serve a connection over.
trait DuplexStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> DuplexStream for T {}

/// Tracks in-flight per-connection tasks so a caller can wait for them to
/// finish on their own (see [`drain_connection_tasks`]) instead of
/// abandoning a still-running transfer mid-copy when the accept loop is
/// aborted or the process is about to exit.
pub type ConnectionTasks = Arc<AsyncMutex<JoinSet<()>>>;

pub fn new_connection_tasks() -> ConnectionTasks {
    Arc::new(AsyncMutex::new(JoinSet::new()))
}

/// Waits, up to `grace_period`, for every currently tracked connection task
/// to finish. Once the local end of an `ssh` session's tunnel is gone, an
/// in-flight transfer's next socket read/write fails almost immediately and
/// its own error-path cleanup (e.g. removing a partial temp file) runs to
/// completion here rather than being cut off by `agent_handle.abort()` or
/// process exit. Not a hard guarantee — a connection that never notices the
/// tunnel is gone is simply abandoned once the grace period elapses, rather
/// than blocking shutdown forever.
pub async fn drain_connection_tasks(tasks: &ConnectionTasks, grace_period: Duration) {
    let mut set = tasks.lock().await;
    let _ = tokio::time::timeout(grace_period, async {
        while set.join_next().await.is_some() {}
    })
    .await;
}

/// The local end of the session's transfer channel: a Unix socket
/// (macOS/Linux) or a loopback TCP socket (Windows). `tokio::net::UnixListener`
/// only exists on unix targets, hence the `Unix` variant is cfg-gated.
pub enum LocalListener {
    #[cfg(unix)]
    Unix(UnixListener),
    // Only constructed on Windows (see `ssh::bind_local_listener`); a
    // non-Windows build never builds that constructor, hence the allow.
    #[cfg_attr(not(windows), allow(dead_code))]
    Tcp(TcpListener),
}

impl LocalListener {
    /// Runs the accept loop until the listener is closed (session
    /// teardown). Each connection is handled on its own task tracked in
    /// `tasks` (see [`drain_connection_tasks`]); a failed connection is
    /// logged and does not bring down the agent or the session.
    pub async fn serve(self, token: String, session_local_dir: PathBuf, tasks: ConnectionTasks) {
        match self {
            #[cfg(unix)]
            LocalListener::Unix(listener) => loop {
                let stream = match listener.accept().await {
                    Ok((stream, _addr)) => stream,
                    Err(_) => return,
                };
                spawn_connection(stream, token.clone(), session_local_dir.clone(), &tasks).await;
            },
            LocalListener::Tcp(listener) => loop {
                let stream = match listener.accept().await {
                    Ok((stream, _addr)) => stream,
                    Err(_) => return,
                };
                spawn_connection(stream, token.clone(), session_local_dir.clone(), &tasks).await;
            },
        }
    }
}

async fn spawn_connection(
    stream: impl DuplexStream,
    token: String,
    session_local_dir: PathBuf,
    tasks: &ConnectionTasks,
) {
    tasks.lock().await.spawn(async move {
        if let Err(error) = handle_connection(stream, &token, &session_local_dir).await {
            eprintln!("shine ssh: transfer agent connection error: {error:#}");
        }
    });
}

async fn handle_connection(
    mut stream: impl DuplexStream,
    token: &str,
    session_local_dir: &Path,
) -> Result<()> {
    let hello: ClientMessage = protocol::read_message(&mut stream).await?;
    let ClientMessage::Hello { protocol_version } = hello else {
        bail!("expected a Hello message to open the connection");
    };
    if protocol_version != PROTOCOL_VERSION {
        protocol::write_message(
            &mut stream,
            &ServerMessage::Error {
                message: format!(
                    "protocol version mismatch: local agent speaks v{PROTOCOL_VERSION}, remote shine speaks v{protocol_version}; upgrade whichever side is older"
                ),
            },
        )
        .await?;
        return Ok(());
    }
    protocol::write_message(
        &mut stream,
        &ServerMessage::HelloAck {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await?;

    let request: ClientMessage = protocol::read_message(&mut stream).await?;
    match request {
        ClientMessage::Hello { .. } => bail!("unexpected second Hello message"),
        ClientMessage::PutFile {
            token: request_token,
            dest_hint,
            filename,
            is_dir,
            size,
            force,
            dry_run,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            handle_put_file(
                &mut stream,
                session_local_dir,
                PutFileRequest {
                    dest_hint: dest_hint.as_deref(),
                    filename: &filename,
                    is_dir,
                    size,
                    force,
                    dry_run,
                },
            )
            .await
        }
        ClientMessage::GetFile {
            token: request_token,
            source_hint,
            dry_run,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            handle_get_file(&mut stream, session_local_dir, &source_hint, dry_run).await
        }
        ClientMessage::Status {
            token: request_token,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            protocol::write_message(
                &mut stream,
                &ServerMessage::StatusResponse {
                    session_local_dir: session_local_dir.display().to_string(),
                },
            )
            .await
        }
    }
}

async fn send_error(stream: &mut impl DuplexStream, message: impl Into<String>) -> Result<()> {
    protocol::write_message(
        stream,
        &ServerMessage::Error {
            message: message.into(),
        },
    )
    .await
}

/// Bundles `PutFile` request fields to keep `handle_put_file`'s parameter
/// list readable (and under clippy's `too_many_arguments` threshold).
struct PutFileRequest<'a> {
    dest_hint: Option<&'a str>,
    filename: &'a str,
    is_dir: bool,
    size: u64,
    force: bool,
    dry_run: bool,
}

async fn handle_put_file(
    stream: &mut impl DuplexStream,
    session_local_dir: &Path,
    request: PutFileRequest<'_>,
) -> Result<()> {
    let PutFileRequest {
        dest_hint,
        filename,
        is_dir,
        size,
        force,
        dry_run,
    } = request;

    let resolved = match resolve_target_path(session_local_dir, dest_hint, filename) {
        Ok(path) => path,
        Err(error) => return send_error(stream, error.to_string()).await,
    };

    let destination_is_dir = resolved.is_dir();
    let would_overwrite = resolved.exists();
    if is_dir {
        if would_overwrite && !destination_is_dir {
            return send_error(
                stream,
                format!(
                    "refusing to overwrite a file with a directory: {}",
                    resolved.display()
                ),
            )
            .await;
        }
        if destination_is_dir && !force {
            return send_error(
                stream,
                format!(
                    "destination directory already exists (pass --force to merge into it): {}",
                    resolved.display()
                ),
            )
            .await;
        }
    } else {
        if destination_is_dir {
            return send_error(
                stream,
                format!(
                    "refusing to overwrite a directory with a file: {}",
                    resolved.display()
                ),
            )
            .await;
        }
        if would_overwrite && !force {
            return send_error(
                stream,
                format!(
                    "destination already exists (pass --force to overwrite): {}",
                    resolved.display()
                ),
            )
            .await;
        }
    }
    let Some(parent) = resolved.parent() else {
        return send_error(stream, "destination has no parent directory").await;
    };
    if !parent.is_dir() {
        return send_error(
            stream,
            format!("destination directory does not exist: {}", parent.display()),
        )
        .await;
    }

    if dry_run {
        return protocol::write_message(
            stream,
            &ServerMessage::Preview {
                resolved_path: resolved.display().to_string(),
                is_dir,
                size: None,
                would_overwrite,
            },
        )
        .await;
    }

    protocol::write_message(stream, &ServerMessage::Proceed).await?;

    if is_dir {
        let temp_tar_path =
            std::env::temp_dir().join(format!("shine-ssh-put-dir-{}.tar", uuid::Uuid::new_v4()));
        let mut temp_file = tokio::fs::File::create(&temp_tar_path)
            .await
            .with_context(|| format!("creating temp file {}", temp_tar_path.display()))?;
        let copy_result = protocol::copy_exact(stream, &mut temp_file, size).await;
        if let Err(error) = copy_result {
            let _ = tokio::fs::remove_file(&temp_tar_path).await;
            return Err(error);
        }
        temp_file
            .sync_all()
            .await
            .context("failed to sync received tar archive to disk")?;
        drop(temp_file);

        let extract_result =
            dir_transfer::extract_tar_from_file(temp_tar_path.clone(), resolved.clone()).await;
        let _ = tokio::fs::remove_file(&temp_tar_path).await;
        if let Err(error) = extract_result {
            return send_error(stream, format!("{error:#}")).await;
        }

        return protocol::write_message(
            stream,
            &ServerMessage::PutAck {
                resolved_path: resolved.display().to_string(),
                bytes_written: size,
            },
        )
        .await;
    }

    let temp_path = parent.join(format!(".shine-ssh-put-{}", uuid::Uuid::new_v4()));
    let mut temp_file = tokio::fs::File::create(&temp_path)
        .await
        .with_context(|| format!("creating temp file {}", temp_path.display()))?;
    let copy_result = protocol::copy_exact(stream, &mut temp_file, size).await;
    if let Err(error) = copy_result {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error);
    }
    temp_file
        .sync_all()
        .await
        .context("failed to sync written file to disk")?;
    drop(temp_file);

    if let Err(error) = tokio::fs::rename(&temp_path, &resolved).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error).with_context(|| format!("finalizing {}", resolved.display()));
    }

    protocol::write_message(
        stream,
        &ServerMessage::PutAck {
            resolved_path: resolved.display().to_string(),
            bytes_written: size,
        },
    )
    .await
}

async fn handle_get_file(
    stream: &mut impl DuplexStream,
    session_local_dir: &Path,
    source_hint: &str,
    dry_run: bool,
) -> Result<()> {
    let resolved = match resolve_source_path(session_local_dir, source_hint) {
        Ok(path) => path,
        Err(error) => return send_error(stream, error.to_string()).await,
    };

    let metadata = match tokio::fs::metadata(&resolved).await {
        Ok(metadata) => metadata,
        Err(error) => {
            return send_error(
                stream,
                format!("cannot read {}: {error}", resolved.display()),
            )
            .await;
        }
    };

    if metadata.is_dir() {
        if dry_run {
            return protocol::write_message(
                stream,
                &ServerMessage::Preview {
                    resolved_path: resolved.display().to_string(),
                    is_dir: true,
                    size: None,
                    would_overwrite: false,
                },
            )
            .await;
        }

        let (tar_path, tar_size) = dir_transfer::build_tar_to_temp_file(resolved.clone()).await?;
        let send_result: Result<()> = async {
            let mut tar_file = tokio::fs::File::open(&tar_path)
                .await
                .with_context(|| format!("opening {}", tar_path.display()))?;
            protocol::write_message(
                stream,
                &ServerMessage::GetHeader {
                    resolved_path: resolved.display().to_string(),
                    is_dir: true,
                    size: tar_size,
                },
            )
            .await?;
            protocol::copy_exact(&mut tar_file, stream, tar_size).await?;
            stream.flush().await.context("failed to flush socket")?;
            Ok(())
        }
        .await;
        let _ = tokio::fs::remove_file(&tar_path).await;
        return send_result;
    }

    if dry_run {
        return protocol::write_message(
            stream,
            &ServerMessage::Preview {
                resolved_path: resolved.display().to_string(),
                is_dir: false,
                size: Some(metadata.len()),
                would_overwrite: false,
            },
        )
        .await;
    }

    let mut file = tokio::fs::File::open(&resolved)
        .await
        .with_context(|| format!("opening {}", resolved.display()))?;
    protocol::write_message(
        stream,
        &ServerMessage::GetHeader {
            resolved_path: resolved.display().to_string(),
            is_dir: false,
            size: metadata.len(),
        },
    )
    .await?;
    protocol::copy_exact(&mut file, stream, metadata.len()).await?;
    stream.flush().await.context("failed to flush socket")?;
    Ok(())
}

/// Resolves a destination path per docs/ssh-local-transfer-prd.md section 6.1:
/// an omitted hint defaults to `base_dir`; an existing-directory candidate
/// gets the source filename appended; anything else is used verbatim.
///
/// `filename` is meant to be a bare basename (the honest client derives it
/// via `Path::file_name()`), never a caller-chosen destination — unlike
/// `hint`, it must not be absolute or escape the resolved directory
/// candidate via `..` or an embedded path separator.
fn resolve_target_path(base_dir: &Path, hint: Option<&str>, filename: &str) -> Result<PathBuf> {
    ensure_single_path_component(filename)?;
    let candidate = match hint {
        None => base_dir.to_path_buf(),
        Some(raw) => expand_hint(base_dir, raw),
    };
    Ok(if candidate.is_dir() {
        candidate.join(filename)
    } else {
        candidate
    })
}

/// Rejects anything but a single normal path component, so a `filename`
/// (which should only ever be a basename) can't be used to escape the
/// resolved destination directory via `..`, an absolute path, or an
/// embedded separator.
fn ensure_single_path_component(filename: &str) -> Result<()> {
    use std::path::Component;
    match Path::new(filename)
        .components()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [Component::Normal(_)] => Ok(()),
        _ => bail!("invalid filename {filename:?}: must be a single path component"),
    }
}

/// Resolves a source path: always relative to `base_dir` unless absolute
/// (or `~`-prefixed, expanded against the local home directory).
fn resolve_source_path(base_dir: &Path, hint: &str) -> Result<PathBuf> {
    Ok(expand_hint(base_dir, hint))
}

/// Expands a wire-supplied hint (`dest_hint`/`source_hint`) against `base_dir`.
///
/// Deliberately uses `tilde_expand` (leading `~` only) rather than
/// `home::full_expand`: those hints come from the remote peer over the
/// transfer protocol, and full shell-style `${VAR}` substitution would let a
/// forged request pull arbitrary values out of the *local* agent process's
/// own environment into a filesystem path.
fn expand_hint(base_dir: &Path, raw: &str) -> PathBuf {
    let expanded = home::tilde_expand(raw);
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("shine-ssh-agent-test-{}", uuid::Uuid::new_v4()));
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

    #[test]
    fn omitted_hint_defaults_to_base_dir_plus_filename() {
        let dir = TempDir::new();
        let resolved = resolve_target_path(dir.path(), None, "result.log").unwrap();
        assert_eq!(resolved, dir.path().join("result.log"));
    }

    #[test]
    fn hint_pointing_at_existing_directory_appends_filename() {
        let dir = TempDir::new();
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        let resolved = resolve_target_path(dir.path(), Some("subdir/"), "result.log").unwrap();
        assert_eq!(resolved, subdir.join("result.log"));
    }

    #[test]
    fn hint_pointing_at_new_path_is_used_verbatim() {
        let dir = TempDir::new();
        let resolved = resolve_target_path(dir.path(), Some("renamed.log"), "result.log").unwrap();
        assert_eq!(resolved, dir.path().join("renamed.log"));
    }

    #[test]
    fn absolute_hint_ignores_base_dir() {
        let dir = TempDir::new();
        let absolute = dir.path().join("elsewhere.log");
        let resolved =
            resolve_target_path(dir.path(), Some(absolute.to_str().unwrap()), "result.log")
                .unwrap();
        assert_eq!(resolved, absolute);
    }

    #[test]
    fn source_hint_relative_to_base_dir() {
        let dir = TempDir::new();
        let resolved = resolve_source_path(dir.path(), "notes.txt").unwrap();
        assert_eq!(resolved, dir.path().join("notes.txt"));
    }

    #[test]
    fn parent_traversal_filename_is_rejected() {
        let dir = TempDir::new();
        let error = resolve_target_path(dir.path(), None, "../../.ssh/authorized_keys")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("invalid filename"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn absolute_filename_is_rejected() {
        let dir = TempDir::new();
        let error = resolve_target_path(dir.path(), None, "/etc/passwd")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("invalid filename"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn embedded_separator_filename_is_rejected() {
        let dir = TempDir::new();
        let error = resolve_target_path(dir.path(), None, "subdir/result.log")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("invalid filename"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn bare_filename_is_accepted() {
        assert!(ensure_single_path_component("result.log").is_ok());
    }
}
