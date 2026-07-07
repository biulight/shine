//! `shine local download/upload`: runs on the remote host inside a
//! `shine ssh` session. Dials the socket forwarded back to the local
//! transfer agent (see `agent.rs`) and speaks the protocol in `protocol.rs`.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

use super::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use crate::home;

struct SessionEnv {
    token: String,
    remote_sock: String,
}

fn session_from_env() -> Result<SessionEnv> {
    let session = std::env::var("SHINE_SSH_SESSION").ok();
    let token = std::env::var("SHINE_SSH_TOKEN").ok();
    let remote_sock = std::env::var("SHINE_SSH_REMOTE_SOCK").ok();
    let (Some(_session), Some(token), Some(remote_sock)) = (session, token, remote_sock) else {
        bail!(
            "this shell is not inside a `shine ssh` session (SHINE_SSH_SESSION/SHINE_SSH_TOKEN/SHINE_SSH_REMOTE_SOCK are not set); run `shine ssh <host>` first"
        );
    };
    Ok(SessionEnv { token, remote_sock })
}

async fn connect_and_handshake(remote_sock: &str) -> Result<UnixStream> {
    let mut stream = UnixStream::connect(remote_sock).await.with_context(|| {
        format!(
            "could not reach the local shine transfer agent through the forwarded SSH connection at {remote_sock}; is the `shine ssh` session this shell was started under still alive?"
        )
    })?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await?;
    let ack: ServerMessage = protocol::read_message(&mut stream).await?;
    match ack {
        ServerMessage::HelloAck { protocol_version } if protocol_version == PROTOCOL_VERSION => {}
        ServerMessage::HelloAck { protocol_version } => {
            bail!(
                "protocol version mismatch: this shine speaks v{PROTOCOL_VERSION}, the local agent speaks v{protocol_version}; upgrade whichever side is older"
            );
        }
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected handshake response: {other:?}"),
    }
    Ok(stream)
}

/// Resolves a remote-side path argument against the current directory,
/// expanding `~` against the remote host's own home directory.
fn resolve_remote_path(raw: &str) -> Result<PathBuf> {
    let expanded = home::full_expand(raw).with_context(|| format!("expanding path {raw:?}"))?;
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .context("reading current directory")?
        .join(path))
}

pub async fn handle_download(
    remote_source: &str,
    local_destination: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let session = session_from_env()?;
    let resolved_source = resolve_remote_path(remote_source)?;
    let metadata = tokio::fs::metadata(&resolved_source)
        .await
        .with_context(|| format!("cannot read {}", resolved_source.display()))?;
    if metadata.is_dir() {
        bail!(
            "{} is a directory; directory transfers are not supported yet",
            resolved_source.display()
        );
    }
    let filename = resolved_source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("remote source has no file name")?;

    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: session.token.clone(),
            dest_hint: local_destination.map(str::to_string),
            filename,
            size: metadata.len(),
            force,
            dry_run,
        },
    )
    .await?;

    let response: ServerMessage = protocol::read_message(&mut stream).await?;
    match response {
        ServerMessage::Preview {
            resolved_path,
            would_overwrite,
            ..
        } => {
            println!(
                "Would download {} ({})\n  remote: {}\n  local:  {}",
                human_bytes(metadata.len()),
                if would_overwrite {
                    "would overwrite existing file"
                } else {
                    "new file"
                },
                resolved_source.display(),
                resolved_path
            );
            Ok(())
        }
        ServerMessage::Proceed => {
            let mut file = tokio::fs::File::open(&resolved_source)
                .await
                .with_context(|| format!("opening {}", resolved_source.display()))?;
            protocol::copy_exact(&mut file, &mut stream, metadata.len()).await?;
            let ack: ServerMessage = protocol::read_message(&mut stream).await?;
            match ack {
                ServerMessage::PutAck {
                    resolved_path,
                    bytes_written,
                } => {
                    println!(
                        "Downloaded {}\n  remote: {}\n  local:  {}",
                        human_bytes(bytes_written),
                        resolved_source.display(),
                        resolved_path
                    );
                    Ok(())
                }
                ServerMessage::Error { message } => bail!("{message}"),
                other => bail!("unexpected response: {other:?}"),
            }
        }
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

pub async fn handle_upload(
    local_source: &str,
    remote_destination: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let session = session_from_env()?;

    let cwd = std::env::current_dir().context("reading current directory")?;
    let dest_candidate = match remote_destination {
        None => cwd.clone(),
        Some(raw) => {
            let expanded =
                home::full_expand(raw).with_context(|| format!("expanding path {raw:?}"))?;
            let path = PathBuf::from(expanded);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        }
    };
    let source_basename = Path::new(local_source)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .with_context(|| format!("{local_source:?} has no file name"))?;
    let resolved_dest = if dest_candidate.is_dir() {
        dest_candidate.join(&source_basename)
    } else {
        dest_candidate
    };

    if resolved_dest.is_dir() {
        bail!(
            "refusing to overwrite a directory with a file: {}",
            resolved_dest.display()
        );
    }
    let would_overwrite = resolved_dest.exists();
    if would_overwrite && !force && !dry_run {
        bail!(
            "destination already exists (pass --force to overwrite): {}",
            resolved_dest.display()
        );
    }

    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::GetFile {
            token: session.token.clone(),
            source_hint: local_source.to_string(),
            dry_run,
        },
    )
    .await?;

    let response: ServerMessage = protocol::read_message(&mut stream).await?;
    match response {
        ServerMessage::Preview {
            resolved_path,
            size,
            ..
        } => {
            println!(
                "Would upload {} ({})\n  local:  {}\n  remote: {}",
                size.map(human_bytes).unwrap_or_default(),
                if would_overwrite {
                    "would overwrite existing file"
                } else {
                    "new file"
                },
                resolved_path,
                resolved_dest.display()
            );
            Ok(())
        }
        ServerMessage::GetHeader {
            resolved_path,
            size,
        } => {
            let Some(parent) = resolved_dest.parent() else {
                bail!("destination has no parent directory");
            };
            if !parent.is_dir() {
                bail!("destination directory does not exist: {}", parent.display());
            }
            let temp_path = parent.join(format!(".shine-ssh-upload-{}", uuid::Uuid::new_v4()));
            let mut temp_file = tokio::fs::File::create(&temp_path)
                .await
                .with_context(|| format!("creating temp file {}", temp_path.display()))?;
            let copy_result = protocol::copy_exact(&mut stream, &mut temp_file, size).await;
            if let Err(error) = copy_result {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(error);
            }
            temp_file
                .sync_all()
                .await
                .context("failed to sync written file to disk")?;
            drop(temp_file);
            if let Err(error) = tokio::fs::rename(&temp_path, &resolved_dest).await {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(error)
                    .with_context(|| format!("finalizing {}", resolved_dest.display()));
            }
            println!(
                "Uploaded {}\n  local:  {}\n  remote: {}",
                human_bytes(size),
                resolved_path,
                resolved_dest.display()
            );
            Ok(())
        }
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_small_byte_counts_without_decimals() {
        assert_eq!(human_bytes(512), "512 B");
    }

    #[test]
    fn formats_larger_byte_counts_with_units() {
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 2), "2.0 MiB");
    }
}
