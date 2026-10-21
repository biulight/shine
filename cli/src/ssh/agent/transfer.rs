//! Resolve local operands, select transport, preview and execute a transfer.

mod argv;
mod relay;
#[cfg(test)]
mod tests;

use super::connection::send_error;
use super::{DuplexStream, SessionContext};
use crate::home;
use crate::ssh::protocol::{self, Direction, LogStream, ServerMessage, Tool};
use anyhow::{Context, Result, bail};
use argv::*;
use relay::spawn_and_relay;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Owned form of a `Transfer` request's fields.
pub(super) struct TransferRequest {
    pub(super) direction: Direction,
    pub(super) remote_spec: String,
    pub(super) local_spec: Option<String>,
    pub(super) force: bool,
    pub(super) dry_run: bool,
    pub(super) use_scp: bool,
}

pub(super) async fn handle_transfer(
    stream: &mut impl DuplexStream,
    ctx: &SessionContext,
    req: TransferRequest,
) -> Result<()> {
    // Resolve the local side of the transfer against the session directory.
    let local_paths = match req.direction {
        Direction::Download => {
            resolve_local_dest(ctx, req.local_spec.as_deref()).map(|path| vec![path])
        }
        Direction::Upload => resolve_local_sources(ctx, req.local_spec.as_deref()),
    };
    let local_paths = match local_paths {
        Ok(paths) => paths,
        Err(error) => return send_error(stream, error.to_string()).await,
    };

    // Pick the tool: honor --scp, else prefer rsync but fall back to scp when
    // rsync is missing locally, when it can't be reached on the remote, or when
    // the destination is an IPv6 literal, or the ssh options can't be handed
    // to rsync safely. macOS system rsync splits even bracketed IPv6 operands.
    let (mut tool, mut fell_back, mut note) = choose_tool(req.use_scp, rsync_available().await);
    if tool == Tool::Rsync && ipv6_destination(&ctx.host).is_some() {
        tool = Tool::Scp;
        fell_back = true;
        note = Some("using scp for IPv6 compatibility with system rsync".to_string());
    }
    if tool == Tool::Rsync && rsync_rsh(ctx).is_err() {
        tool = Tool::Scp;
        fell_back = true;
        note = Some("ssh options can't be passed to rsync safely; using scp".to_string());
    }
    if tool == Tool::Rsync
        && ctx.control_path.is_some()
        && remote_rsync_available(ctx).await == Some(false)
    {
        tool = Tool::Scp;
        fell_back = true;
        note = Some("rsync not found on the remote host; using scp".to_string());
    }

    // scp has no dry-run: synthesize a preview instead of spawning anything.
    if req.dry_run && tool == Tool::Scp {
        protocol::write_message(
            stream,
            &ServerMessage::Starting {
                tool,
                fell_back,
                note,
            },
        )
        .await?;
        let dest = describe_transfer(ctx, req.direction, &req.remote_spec, &local_paths);
        protocol::write_message(
            stream,
            &ServerMessage::Log {
                stream: LogStream::Stdout,
                chunk: format!("Would copy {dest} via scp (no byte-accurate preview)\n"),
            },
        )
        .await?;
        return protocol::write_message(stream, &ServerMessage::Done { code: 0 }).await;
    }

    let plan = TransferPlan {
        direction: req.direction,
        remote_spec: req.remote_spec,
        local_paths,
        force: req.force,
        dry_run: req.dry_run,
    };
    let command = match build_transfer_argv(tool, ctx, &plan) {
        Ok(command) => command,
        Err(error) => return send_error(stream, error.to_string()).await,
    };

    protocol::write_message(
        stream,
        &ServerMessage::Starting {
            tool,
            fell_back,
            note,
        },
    )
    .await?;

    // rsync gets --ignore-existing when the user didn't pass --force, so it
    // never clobbers; scp can't gate overwrite at all, so warn instead.
    if tool == Tool::Scp && !plan.force && !plan.dry_run {
        protocol::write_message(
            stream,
            &ServerMessage::Log {
                stream: LogStream::Stderr,
                chunk: "shine: scp cannot enforce overwrite protection; existing files may be overwritten\n".to_string(),
            },
        )
        .await?;
    }

    let code = spawn_and_relay(stream, command).await?;
    protocol::write_message(stream, &ServerMessage::Done { code }).await
}

/// Resolves a download destination: an omitted hint defaults to the session's
/// local directory; otherwise the wire hint is expanded (tilde-only) and
/// anchored to it.
fn resolve_local_dest(ctx: &SessionContext, hint: Option<&str>) -> Result<PathBuf> {
    match hint {
        None => {
            ctx.local_dir
                .to_str()
                .context("local transfer path is not valid UTF-8")?;
            Ok(ctx.local_dir.clone())
        }
        Some(raw) => expand_hint(&ctx.local_dir, raw),
    }
}

/// Resolves an upload source, expanding a local glob (`*`, `?`, `[...]`) against
/// the filesystem when present. Returns every matched path; a glob that matches
/// nothing is an error rather than a silent no-op.
fn resolve_local_sources(ctx: &SessionContext, hint: Option<&str>) -> Result<Vec<PathBuf>> {
    let raw = hint.context("upload requires a source path")?;
    let expanded = expand_hint(&ctx.local_dir, raw)?;
    if !has_glob_metacharacters(raw) {
        return Ok(vec![expanded]);
    }
    let pattern = expanded
        .to_str()
        .context("local transfer path is not valid UTF-8")?;
    let mut matches = Vec::new();
    for entry in glob::glob(pattern).with_context(|| format!("invalid glob pattern {raw:?}"))? {
        matches.push(entry.context("error while matching glob")?);
    }
    if matches.is_empty() {
        bail!("no local files match {raw:?}");
    }
    Ok(matches)
}

fn has_glob_metacharacters(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// Expands a wire-supplied hint against `base_dir`.
///
/// Deliberately expands only a leading `~` rather than using
/// `home::full_expand`: these hints come from the remote peer over the transfer
/// protocol, and full shell-style `${VAR}` substitution would let a forged
/// request pull arbitrary values out of the *local* agent process's own
/// environment into a filesystem path.
fn expand_hint(base_dir: &Path, raw: &str) -> Result<PathBuf> {
    let home = home::effective_home_dir();
    let mut unrepresentable_home = false;
    let expanded = shellexpand::tilde_with_context(raw, || {
        let value = home.to_str();
        unrepresentable_home = value.is_none();
        value
    });
    if unrepresentable_home {
        bail!("local transfer home directory is not valid UTF-8");
    }
    let path = PathBuf::from(expanded.as_ref());
    let path = if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    };
    path.to_str()
        .context("local transfer path is not valid UTF-8")?;
    Ok(path)
}

/// Probes whether rsync is runnable locally (deterministic ENOENT check).
async fn rsync_available() -> bool {
    let bin = resolve_tool_binary(Tool::Rsync);
    tokio::process::Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Probes whether rsync exists on the remote host, reusing the control master
/// so it costs no extra authentication. Returns `None` when we can't cheaply
/// probe (no control master) so the caller skips the check rather than opening
/// a second authenticated connection just to look.
async fn remote_rsync_available(ctx: &SessionContext) -> Option<bool> {
    let control_path = ctx.control_path.as_ref()?;
    let ok = tokio::process::Command::new("ssh")
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg("ControlMaster=no")
        .arg("--")
        .arg(&ctx.host)
        .arg("command -v rsync >/dev/null 2>&1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(true); // On probe failure, assume present and let rsync report.
    Some(ok)
}

/// A short human description of a transfer, for the scp dry-run preview.
fn describe_transfer(
    ctx: &SessionContext,
    direction: Direction,
    remote_spec: &str,
    local_paths: &[PathBuf],
) -> String {
    let remote = format!("{}:{}", transfer_host(&ctx.host), remote_spec);
    // Display-only quoting keeps each operand distinct and escapes control characters.
    let remote = format!("{remote:?}");
    let local = local_paths
        .iter()
        .map(|p| format!("{:?}", p.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    match direction {
        Direction::Download => format!("{remote} -> {local}"),
        Direction::Upload => format!("{local} -> {remote}"),
    }
}
