//! Local transfer agent: the Unix-socket server side of a `shine ssh`
//! session. Listens on the session's local socket (the far end of the SSH
//! `-R` forward) and serves `PutFile` (download) / `GetFile` (upload)
//! requests from the remote `shine local` client. Directories are handled
//! by staging/extracting a tar archive (see `dir_transfer.rs`).

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};

use super::dir_transfer;
use super::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use crate::home;

/// Runs the accept loop until the listener is closed (session teardown).
/// Each connection is handled on its own task; a failed connection is
/// logged and does not bring down the agent or the session.
pub async fn serve(listener: UnixListener, token: String, session_local_dir: PathBuf) {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let token = token.clone();
        let session_local_dir = session_local_dir.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, &token, &session_local_dir).await {
                eprintln!("shine ssh: transfer agent connection error: {error:#}");
            }
        });
    }
}

async fn handle_connection(
    mut stream: UnixStream,
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

async fn send_error(stream: &mut UnixStream, message: impl Into<String>) -> Result<()> {
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
    stream: &mut UnixStream,
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
    stream: &mut UnixStream,
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
fn resolve_target_path(base_dir: &Path, hint: Option<&str>, filename: &str) -> Result<PathBuf> {
    let candidate = match hint {
        None => base_dir.to_path_buf(),
        Some(raw) => expand_hint(base_dir, raw)?,
    };
    Ok(if candidate.is_dir() {
        candidate.join(filename)
    } else {
        candidate
    })
}

/// Resolves a source path: always relative to `base_dir` unless absolute
/// (or `~`-prefixed, expanded against the local home directory).
fn resolve_source_path(base_dir: &Path, hint: &str) -> Result<PathBuf> {
    expand_hint(base_dir, hint)
}

fn expand_hint(base_dir: &Path, raw: &str) -> Result<PathBuf> {
    let expanded = home::full_expand(raw).with_context(|| format!("expanding path {raw:?}"))?;
    let path = PathBuf::from(expanded);
    Ok(if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    })
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
}
