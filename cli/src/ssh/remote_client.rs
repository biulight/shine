//! `shine local download/upload`: runs on the remote host inside a
//! `shine ssh` session. Dials the socket forwarded back to the local
//! transfer agent (see `agent.rs`) and speaks the protocol in `protocol.rs`.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

use super::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use crate::home;

struct SessionEnv {
    session_id: String,
    token: String,
    remote_sock: String,
}

fn session_from_env() -> Result<SessionEnv> {
    let session_id = std::env::var("SHINE_SSH_SESSION").ok();
    let token = std::env::var("SHINE_SSH_TOKEN").ok();
    let remote_sock = std::env::var("SHINE_SSH_REMOTE_SOCK").ok();
    let (Some(session_id), Some(token), Some(remote_sock)) = (session_id, token, remote_sock)
    else {
        bail!(
            "this shell is not inside a `shine ssh` session (SHINE_SSH_SESSION/SHINE_SSH_TOKEN/SHINE_SSH_REMOTE_SOCK are not set); run `shine ssh <host>` first"
        );
    };
    Ok(SessionEnv {
        session_id,
        token,
        remote_sock,
    })
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
    let is_dir = metadata.is_dir();
    let filename = resolved_source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("remote source has no file name")?;

    // Skip staging a tar archive entirely for a directory dry-run preview.
    let (content_path, size): (PathBuf, u64) = if is_dir {
        if dry_run {
            (resolved_source.clone(), 0)
        } else {
            super::dir_transfer::build_tar_to_temp_file(resolved_source.clone()).await?
        }
    } else {
        (resolved_source.clone(), metadata.len())
    };

    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::PutFile {
            token: session.token.clone(),
            dest_hint: local_destination.map(str::to_string),
            filename,
            is_dir,
            size,
            force,
            dry_run,
        },
    )
    .await?;

    let response: ServerMessage = protocol::read_message(&mut stream).await?;
    let result: Result<()> = async {
        match response {
            ServerMessage::Preview {
                resolved_path,
                would_overwrite,
                ..
            } => {
                let overwrite_note = match (is_dir, would_overwrite) {
                    (true, true) => "would merge into existing directory",
                    (true, false) => "new directory",
                    (false, true) => "would overwrite existing file",
                    (false, false) => "new file",
                };
                if is_dir {
                    println!(
                        "Would download directory ({overwrite_note})\n  remote: {}\n  local:  {}",
                        resolved_source.display(),
                        resolved_path
                    );
                } else {
                    println!(
                        "Would download {} ({overwrite_note})\n  remote: {}\n  local:  {}",
                        human_bytes(size),
                        resolved_source.display(),
                        resolved_path
                    );
                }
                Ok(())
            }
            ServerMessage::Proceed => {
                let mut file = tokio::fs::File::open(&content_path)
                    .await
                    .with_context(|| format!("opening {}", content_path.display()))?;
                let mut progress = ProgressPrinter::new(
                    if is_dir {
                        "Downloading directory"
                    } else {
                        "Downloading"
                    },
                    size,
                );
                protocol::copy_exact_with_progress(&mut file, &mut stream, size, |copied| {
                    progress.update(copied)
                })
                .await?;
                progress.finish();
                match protocol::read_message(&mut stream).await? {
                    ServerMessage::PutAck {
                        resolved_path,
                        bytes_written,
                    } => {
                        println!(
                            "Downloaded {}{}\n  remote: {}\n  local:  {}",
                            if is_dir { "directory " } else { "" },
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
    .await;

    if is_dir && !dry_run {
        let _ = tokio::fs::remove_file(&content_path).await;
    }
    result
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

    // The local source's type (file vs directory) is only known once the
    // agent resolves and stats it below; this only rejects the type-agnostic
    // "exists but no --force" case early to save a round trip in the common
    // case. Type-mismatch checks (file vs directory) happen after the
    // agent's response reveals which one it actually is.
    let destination_is_dir = resolved_dest.is_dir();
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
            is_dir,
            size,
            ..
        } => {
            let overwrite_note = match (is_dir, would_overwrite) {
                (true, true) => "would merge into existing directory",
                (true, false) => "new directory",
                (false, true) => "would overwrite existing file",
                (false, false) => "new file",
            };
            if is_dir {
                println!(
                    "Would upload directory ({overwrite_note})\n  local:  {resolved_path}\n  remote: {}",
                    resolved_dest.display()
                );
            } else {
                println!(
                    "Would upload {} ({overwrite_note})\n  local:  {resolved_path}\n  remote: {}",
                    size.map(human_bytes).unwrap_or_default(),
                    resolved_dest.display()
                );
            }
            Ok(())
        }
        ServerMessage::GetHeader {
            resolved_path,
            is_dir,
            size,
        } => {
            if destination_is_dir && !is_dir {
                bail!(
                    "refusing to overwrite a directory with a file: {}",
                    resolved_dest.display()
                );
            }
            if !destination_is_dir && would_overwrite && is_dir {
                bail!(
                    "refusing to overwrite a file with a directory: {}",
                    resolved_dest.display()
                );
            }

            if is_dir {
                let temp_tar_path = std::env::temp_dir()
                    .join(format!("shine-ssh-upload-dir-{}.tar", uuid::Uuid::new_v4()));
                let mut temp_file = tokio::fs::File::create(&temp_tar_path)
                    .await
                    .with_context(|| format!("creating temp file {}", temp_tar_path.display()))?;
                let mut progress = ProgressPrinter::new("Uploading directory", size);
                let copy_result = protocol::copy_exact_with_progress(
                    &mut stream,
                    &mut temp_file,
                    size,
                    |copied| progress.update(copied),
                )
                .await;
                progress.finish();
                if let Err(error) = copy_result {
                    let _ = tokio::fs::remove_file(&temp_tar_path).await;
                    return Err(error);
                }
                temp_file
                    .sync_all()
                    .await
                    .context("failed to sync received tar archive to disk")?;
                drop(temp_file);

                let extract_result = super::dir_transfer::extract_tar_from_file(
                    temp_tar_path.clone(),
                    resolved_dest.clone(),
                )
                .await;
                let _ = tokio::fs::remove_file(&temp_tar_path).await;
                extract_result?;

                println!(
                    "Uploaded directory {}\n  local:  {resolved_path}\n  remote: {}",
                    human_bytes(size),
                    resolved_dest.display()
                );
                return Ok(());
            }

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
            let mut progress = ProgressPrinter::new("Uploading", size);
            let copy_result =
                protocol::copy_exact_with_progress(&mut stream, &mut temp_file, size, |copied| {
                    progress.update(copied)
                })
                .await;
            progress.finish();
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
                "Uploaded {}\n  local:  {resolved_path}\n  remote: {}",
                human_bytes(size),
                resolved_dest.display()
            );
            Ok(())
        }
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

pub async fn handle_status() -> Result<()> {
    let session = session_from_env()?;

    match connect_and_handshake(&session.remote_sock).await {
        Ok(mut stream) => {
            protocol::write_message(
                &mut stream,
                &ClientMessage::Status {
                    token: session.token.clone(),
                },
            )
            .await?;
            match protocol::read_message(&mut stream).await? {
                ServerMessage::StatusResponse { session_local_dir } => {
                    println!("session:    {}", session.session_id);
                    println!("connection: connected");
                    println!("protocol:   v{PROTOCOL_VERSION}");
                    println!("local dir:  {session_local_dir}");
                    Ok(())
                }
                ServerMessage::Error { message } => bail!("{message}"),
                other => bail!("unexpected response: {other:?}"),
            }
        }
        Err(error) => {
            println!("session:    {}", session.session_id);
            println!("connection: unreachable ({error:#})");
            Ok(())
        }
    }
}

/// Prints a periodic, throttled progress line to stderr while a transfer is
/// in flight — but only when stderr is an attended terminal (PRD section 7:
/// non-TTY environments get a stable single-line result, no live redraw).
struct ProgressPrinter {
    label: &'static str,
    total: u64,
    start: std::time::Instant,
    last_print: std::time::Instant,
    enabled: bool,
}

impl ProgressPrinter {
    fn new(label: &'static str, total: u64) -> Self {
        let now = std::time::Instant::now();
        Self {
            label,
            total,
            start: now,
            last_print: now,
            enabled: total > 0 && console::user_attended_stderr(),
        }
    }

    fn update(&mut self, copied: u64) {
        if !self.enabled {
            return;
        }
        let now = std::time::Instant::now();
        let is_final = copied >= self.total;
        if !is_final && now.duration_since(self.last_print) < std::time::Duration::from_millis(150)
        {
            return;
        }
        self.last_print = now;

        let elapsed = now.duration_since(self.start).as_secs_f64().max(0.001);
        let rate = (copied as f64 / elapsed) as u64;
        let pct = ((copied as f64 / self.total as f64) * 100.0).min(100.0) as u32;
        eprint!(
            "\r{}: {} / {} ({pct}%) \u{2014} {}/s\x1b[K",
            self.label,
            human_bytes(copied),
            human_bytes(self.total),
            human_bytes(rate)
        );
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    fn finish(&self) {
        if self.enabled {
            eprint!("\r\x1b[K");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
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
