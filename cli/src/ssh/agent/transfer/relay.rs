//! Own transfer child/process-group cleanup across I/O errors and cancellation.

use super::argv::TransferCommand;
use crate::ssh::{
    agent::DuplexStream,
    protocol::{self, LogStream, ServerMessage},
};
use anyhow::{Context, Result, bail};
use std::process::Stdio;
use tokio::io::AsyncReadExt;

/// Owns transfer cleanup across relay errors and async cancellation.
struct TransferChild {
    child: tokio::process::Child,
    #[cfg(unix)]
    group: Option<u32>,
}

impl TransferChild {
    fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            // SAFETY: the child was started in its own process group. Keep its ID
            // even after wait reaps it, since descendants may still hold pipes.
            unsafe { libc::kill(-(group as i32), libc::SIGKILL) };
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for TransferChild {
    fn drop(&mut self) {
        // Also cover cancellation while reading, relaying, or waiting.
        self.stop();
    }
}

/// Spawns an argv-only transfer and monitors its control connection through exit.
pub(super) async fn spawn_and_relay(
    stream: &mut impl DuplexStream,
    command: TransferCommand,
) -> Result<i32> {
    let mut process = command.process();
    process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.as_std_mut().process_group(0);
    }
    let child = process
        .spawn()
        .with_context(|| format!("failed to spawn {}", command.program))?;
    let mut transfer = TransferChild {
        #[cfg(unix)]
        group: child.id(),
        child,
    };
    let result = async {
        let mut stdout = transfer
            .child
            .stdout
            .take()
            .context("child stdout was not captured")?;
        let mut stderr = transfer
            .child
            .stderr
            .take()
            .context("child stderr was not captured")?;
        let mut out_buf = vec![0u8; 16 * 1024];
        let mut err_buf = vec![0u8; 16 * 1024];
        let mut connection_buf = [0u8; 1];
        let mut out_open = true;
        let mut err_open = true;
        while out_open || err_open {
            tokio::select! {
                closed = stream.read(&mut connection_buf) => {
                    closed.context("monitoring transfer connection")?;
                    bail!("transfer control connection closed or sent unexpected data");
                }
                result = stdout.read(&mut out_buf), if out_open => {
                    let n = result.context("reading child stdout")?;
                    if n == 0 {
                        out_open = false;
                    } else {
                        relay_chunk(stream, LogStream::Stdout, &out_buf[..n]).await?;
                    }
                }
                result = stderr.read(&mut err_buf), if err_open => {
                    let n = result.context("reading child stderr")?;
                    if n == 0 {
                        err_open = false;
                    } else {
                        relay_chunk(stream, LogStream::Stderr, &err_buf[..n]).await?;
                    }
                }
            }
        }
        tokio::select! {
            status = transfer.child.wait() => {
                Ok(status.context("waiting for transfer child")?.code().unwrap_or(-1))
            }
            closed = stream.read(&mut connection_buf) => {
                closed.context("monitoring transfer connection")?;
                bail!("transfer control connection closed or sent unexpected data");
            }
        }
    }
    .await;
    if result.is_err() {
        transfer.stop();
        let _ = transfer.child.wait().await;
    } else {
        #[cfg(unix)]
        {
            transfer.group = None;
        }
    }
    result
}

async fn relay_chunk(
    stream: &mut impl DuplexStream,
    log_stream: LogStream,
    bytes: &[u8],
) -> Result<()> {
    protocol::write_message(
        stream,
        &ServerMessage::Log {
            stream: log_stream,
            chunk: String::from_utf8_lossy(bytes).into_owned(),
        },
    )
    .await
}
