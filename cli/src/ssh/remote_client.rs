//! `shine local download/upload`: runs on the remote host inside a
//! `shine ssh` session. Dials the socket forwarded back to the local transfer
//! agent (see `agent.rs`) and sends a single `Transfer` request; the local
//! agent runs `rsync`/`scp` and streams its output back, which this process
//! relays to the user's terminal (ADR 0011).

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

use super::protocol::{self, ClientMessage, Direction, LogStream, PROTOCOL_VERSION, ServerMessage};
use crate::env::broker::WorkspaceSnapshot;
use crate::home;

const BROKER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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

pub fn session_available() -> bool {
    std::env::var_os("SHINE_SSH_SESSION").is_some()
        && std::env::var_os("SHINE_SSH_TOKEN").is_some()
        && std::env::var_os("SHINE_SSH_REMOTE_SOCK").is_some()
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

pub async fn request_direct_secrets(
    specs: &[String],
    argv: &[String],
) -> Result<BTreeMap<String, String>> {
    let session = session_from_env()?;
    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::DirectSecret {
            token: session.token,
            specs: specs.to_vec(),
            argv: argv.to_vec(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await?;
    read_secret_response(&mut stream).await
}

pub async fn request_workspace_secrets(
    snapshot: WorkspaceSnapshot,
    argv: &[String],
) -> Result<BTreeMap<String, String>> {
    let session = session_from_env()?;
    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::WorkspaceSecret {
            token: session.token,
            snapshot,
            argv: argv.to_vec(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await?;
    read_secret_response(&mut stream).await
}

async fn read_secret_response(stream: &mut UnixStream) -> Result<BTreeMap<String, String>> {
    match read_broker_message(stream).await? {
        ServerMessage::SecretResponse { values } => Ok(values),
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected secret broker response: {other:?}"),
    }
}

async fn read_broker_message(stream: &mut UnixStream) -> Result<ServerMessage> {
    tokio::time::timeout(BROKER_RESPONSE_TIMEOUT, protocol::read_message(stream))
        .await
        .context("timed out waiting for the local SSH secret broker")?
}

pub async fn describe_workspace(
    snapshot: WorkspaceSnapshot,
    release: &[String],
    argv: &[String],
) -> Result<String> {
    let session = session_from_env()?;
    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::DescribeWorkspace {
            token: session.token,
            snapshot,
            release: release.to_vec(),
            argv: argv.to_vec(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await?;
    match read_broker_message(&mut stream).await? {
        ServerMessage::DescriptionResponse { summary } => Ok(summary),
        ServerMessage::Error { message } => bail!("{message}"),
        other => bail!("unexpected broker description response: {other:?}"),
    }
}

/// Anchors a remote path spec to the remote cwd without disturbing glob
/// metacharacters anywhere in it. `~`/`$VAR` are expanded (the remote trusts its
/// own environment); a relative result is prefixed with the cwd by a string
/// join, never component-canonicalized, so `*`, `?`, `[...]` in any component
/// survive for rsync/scp's remote shell to expand.
fn absolutize_remote_spec(raw: &str) -> Result<String> {
    let expanded = home::full_expand(raw).with_context(|| format!("expanding path {raw:?}"))?;
    if Path::new(&expanded).is_absolute() {
        return Ok(expanded);
    }
    let cwd = std::env::current_dir().context("reading current directory")?;
    Ok(format!("{}/{}", cwd.display(), expanded))
}

pub async fn handle_download(
    remote_source: &str,
    local_destination: Option<&str>,
    force: bool,
    dry_run: bool,
    use_scp: bool,
) -> Result<()> {
    let session = session_from_env()?;
    let remote_spec = absolutize_remote_spec(remote_source)?;
    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: session.token,
            direction: Direction::Download,
            remote_spec,
            local_spec: local_destination.map(str::to_string),
            force,
            dry_run,
            use_scp,
        },
    )
    .await?;
    relay_until_done(&mut stream).await
}

pub async fn handle_upload(
    local_source: &str,
    remote_destination: Option<&str>,
    force: bool,
    dry_run: bool,
    use_scp: bool,
) -> Result<()> {
    let session = session_from_env()?;
    let remote_spec = match remote_destination {
        Some(dest) => absolutize_remote_spec(dest)?,
        None => std::env::current_dir()
            .context("reading current directory")?
            .display()
            .to_string(),
    };
    let mut stream = connect_and_handshake(&session.remote_sock).await?;
    protocol::write_message(
        &mut stream,
        &ClientMessage::Transfer {
            token: session.token,
            direction: Direction::Upload,
            remote_spec,
            local_spec: Some(local_source.to_string()),
            force,
            dry_run,
            use_scp,
        },
    )
    .await?;
    relay_until_done(&mut stream).await
}

/// Reads server frames until `Done`/`Error`, printing relayed rsync/scp output
/// verbatim and propagating the child's exit code as this process's own.
async fn relay_until_done(stream: &mut UnixStream) -> Result<()> {
    loop {
        match protocol::read_message(stream).await? {
            ServerMessage::Starting {
                fell_back, note, ..
            } => {
                if fell_back && let Some(note) = note {
                    eprintln!("shine: {note}");
                }
            }
            ServerMessage::Log {
                stream: which,
                chunk,
            } => match which {
                LogStream::Stdout => {
                    print!("{chunk}");
                    let _ = std::io::stdout().flush();
                }
                LogStream::Stderr => {
                    eprint!("{chunk}");
                    let _ = std::io::stderr().flush();
                }
            },
            ServerMessage::Done { code } => {
                if code != 0 {
                    // Faithfully propagate rsync/scp status (e.g. rsync 23/24
                    // partial-transfer) to any remote script driving this.
                    std::process::exit(code);
                }
                return Ok(());
            }
            ServerMessage::Error { message } => bail!("{message}"),
            other => bail!("unexpected response: {other:?}"),
        }
    }
}

pub async fn handle_status() -> Result<()> {
    let session = session_from_env()?;

    match connect_and_handshake(&session.remote_sock).await {
        Ok(mut stream) => {
            protocol::write_message(
                &mut stream,
                &ClientMessage::Status {
                    token: session.token,
                },
            )
            .await?;
            match protocol::read_message(&mut stream).await? {
                ServerMessage::StatusResponse {
                    session_local_dir,
                    host,
                } => {
                    println!("session:    {}", session.session_id);
                    println!("connection: connected");
                    println!("protocol:   v{PROTOCOL_VERSION}");
                    println!("host:       {host}");
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
