//! Local transfer agent: the server side of a `shine ssh` session's transfer
//! channel (the far end of the SSH `-R` forward). It no longer moves file
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
//! wire.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use super::protocol::{
    self, ClientMessage, Direction, LogStream, PROTOCOL_VERSION, ServerMessage, Tool,
};
use super::session_context::SessionContext;
use crate::home;

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
    pub async fn serve(self, token: String, context: Arc<SessionContext>, tasks: ConnectionTasks) {
        match self {
            #[cfg(unix)]
            LocalListener::Unix(listener) => loop {
                let stream = match listener.accept().await {
                    Ok((stream, _addr)) => stream,
                    Err(_) => return,
                };
                spawn_connection(stream, token.clone(), context.clone(), &tasks).await;
            },
            LocalListener::Tcp(listener) => loop {
                let stream = match listener.accept().await {
                    Ok((stream, _addr)) => stream,
                    Err(_) => return,
                };
                spawn_connection(stream, token.clone(), context.clone(), &tasks).await;
            },
        }
    }
}

async fn spawn_connection(
    stream: impl DuplexStream,
    token: String,
    context: Arc<SessionContext>,
    tasks: &ConnectionTasks,
) {
    tasks.lock().await.spawn(async move {
        if let Err(error) = handle_connection(stream, &token, &context).await {
            eprintln!("shine ssh: transfer agent connection error: {error:#}");
        }
    });
}

async fn handle_connection(
    mut stream: impl DuplexStream,
    token: &str,
    context: &SessionContext,
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
        ClientMessage::Transfer {
            token: request_token,
            direction,
            remote_spec,
            local_spec,
            force,
            dry_run,
            use_scp,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            handle_transfer(
                &mut stream,
                context,
                TransferRequest {
                    direction,
                    remote_spec,
                    local_spec,
                    force,
                    dry_run,
                    use_scp,
                },
            )
            .await
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
                    session_local_dir: context.local_dir.display().to_string(),
                    host: context.host.clone(),
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

/// Owned form of a `Transfer` request's fields.
struct TransferRequest {
    direction: Direction,
    remote_spec: String,
    local_spec: Option<String>,
    force: bool,
    dry_run: bool,
    use_scp: bool,
}

async fn handle_transfer(
    stream: &mut impl DuplexStream,
    ctx: &SessionContext,
    req: TransferRequest,
) -> Result<()> {
    // Resolve the local side of the transfer against the session directory.
    let local_paths = match req.direction {
        Direction::Download => vec![resolve_local_dest(ctx, req.local_spec.as_deref())],
        Direction::Upload => match resolve_local_sources(ctx, req.local_spec.as_deref()) {
            Ok(paths) => paths,
            Err(error) => return send_error(stream, error.to_string()).await,
        },
    };

    // Pick the tool: honor --scp, else prefer rsync but fall back to scp when
    // rsync is missing locally, when it can't be reached on the remote, or when
    // the ssh options can't be handed to rsync safely.
    let (mut tool, mut fell_back, mut note) = choose_tool(req.use_scp, rsync_available().await);
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
fn resolve_local_dest(ctx: &SessionContext, hint: Option<&str>) -> PathBuf {
    match hint {
        None => ctx.local_dir.clone(),
        Some(raw) => expand_hint(&ctx.local_dir, raw),
    }
}

/// Resolves an upload source, expanding a local glob (`*`, `?`, `[...]`) against
/// the filesystem when present. Returns every matched path; a glob that matches
/// nothing is an error rather than a silent no-op.
fn resolve_local_sources(ctx: &SessionContext, hint: Option<&str>) -> Result<Vec<PathBuf>> {
    let raw = hint.context("upload requires a source path")?;
    let expanded = expand_hint(&ctx.local_dir, raw);
    if !has_glob_metacharacters(raw) {
        return Ok(vec![expanded]);
    }
    let pattern = expanded.to_string_lossy();
    let mut matches = Vec::new();
    for entry in glob::glob(&pattern).with_context(|| format!("invalid glob pattern {raw:?}"))? {
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
/// Deliberately uses `tilde_expand` (leading `~` only) rather than
/// `home::full_expand`: these hints come from the remote peer over the transfer
/// protocol, and full shell-style `${VAR}` substitution would let a forged
/// request pull arbitrary values out of the *local* agent process's own
/// environment into a filesystem path.
fn expand_hint(base_dir: &Path, raw: &str) -> PathBuf {
    let expanded = home::tilde_expand(raw);
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

/// A validated, resolved transfer ready to be turned into an argv.
pub(crate) struct TransferPlan {
    pub direction: Direction,
    /// Remote-owned spec (untrusted; already absolutized by the remote client).
    pub remote_spec: String,
    /// Resolved absolute local paths — one destination for download, one or
    /// more sources for upload.
    pub local_paths: Vec<PathBuf>,
    pub force: bool,
    pub dry_run: bool,
}

/// The concrete command to spawn. Program comes from [`resolve_tool_binary`] so
/// tests can substitute a stub script.
pub(crate) struct TransferCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Builds the rsync/scp argv (the pure, unit-tested seam). Never lets wire input
/// reach an option position: the remote operand is always `<host>:<remote_spec>`
/// after a `--`, and the ssh reconnection string is built solely from `ctx`.
pub(crate) fn build_transfer_argv(
    tool: Tool,
    ctx: &SessionContext,
    plan: &TransferPlan,
) -> Result<TransferCommand> {
    if plan.remote_spec.contains(['\0', '\n', '\r']) {
        bail!("remote path contains an illegal control character");
    }
    let remote_operand = format!("{}:{}", ctx.host, plan.remote_spec);
    let local_operands: Vec<String> = plan
        .local_paths
        .iter()
        .map(|path| safe_local_operand(path))
        .collect();
    if local_operands.is_empty() {
        bail!("no local path resolved for the transfer");
    }

    let mut args: Vec<String> = Vec::new();
    match tool {
        Tool::Rsync => {
            args.push("-a".to_string());
            if plan.dry_run {
                args.push("-n".to_string());
                args.push("--itemize-changes".to_string());
            }
            args.push("--info=progress2".to_string());
            if !plan.force {
                // Never clobber without --force. rsync otherwise always overwrites.
                args.push("--ignore-existing".to_string());
            }
            if let Some(rsh) = rsync_rsh(ctx)? {
                args.push("-e".to_string());
                args.push(rsh);
            }
            args.push("--".to_string());
        }
        Tool::Scp => {
            args.push("-r".to_string());
            for opt in scp_connection_options(ctx) {
                args.push(opt);
            }
            args.push("--".to_string());
        }
    }

    match plan.direction {
        Direction::Download => {
            // remote is the source, local the (single) destination.
            args.push(remote_operand);
            args.extend(local_operands);
        }
        Direction::Upload => {
            // local paths are the sources, remote the destination.
            args.extend(local_operands);
            args.push(remote_operand);
        }
    }

    Ok(TransferCommand {
        program: resolve_tool_binary(tool),
        args,
    })
}

/// Renders one resolved local path as a safe operand: absolute paths (the
/// normal case, since we anchor to the session dir) pass through; a relative
/// path that could be mistaken for an option is `./`-prefixed. Always used
/// after a `--` separator as defense in depth.
fn safe_local_operand(path: &Path) -> String {
    let display = path.to_string_lossy().into_owned();
    if display.starts_with('-') {
        format!("./{display}")
    } else {
        display
    }
}

/// The ssh reconnection options, derived **only** from the local session
/// context. When we own a control master, reuse it (no re-auth); otherwise
/// replay the user's own ssh options verbatim.
fn ssh_reconnect_options(ctx: &SessionContext) -> Vec<String> {
    match &ctx.control_path {
        Some(path) => vec![
            "-o".to_string(),
            format!("ControlPath={}", path.display()),
            "-o".to_string(),
            "ControlMaster=no".to_string(),
        ],
        None => ctx.ssh_options.clone(),
    }
}

/// The value for rsync's `-e`/`--rsh`, or `None` when the default (`ssh`) needs
/// no options. rsync splits this string on whitespace with no quote handling,
/// so an option token containing whitespace can't be represented — that case is
/// an error, and the caller falls back to scp.
fn rsync_rsh(ctx: &SessionContext) -> Result<Option<String>> {
    let options = ssh_reconnect_options(ctx);
    if options.is_empty() {
        return Ok(None);
    }
    if options.iter().any(|o| o.chars().any(char::is_whitespace)) {
        bail!("ssh options contain whitespace and can't be passed to rsync -e");
    }
    Ok(Some(format!("ssh {}", options.join(" "))))
}

/// scp connection options: the same reconnection settings, passed as individual
/// argv tokens. (scp reads `~/.ssh/config`, so an alias needs nothing here.)
fn scp_connection_options(ctx: &SessionContext) -> Vec<String> {
    ssh_reconnect_options(ctx)
}

/// Resolves the binary for a tool, honoring a per-tool test-override env var so
/// integration tests can point at a stub script.
pub(crate) fn resolve_tool_binary(tool: Tool) -> String {
    let (var, default) = match tool {
        Tool::Rsync => ("SHINE_TEST_RSYNC", "rsync"),
        Tool::Scp => ("SHINE_TEST_SCP", "scp"),
    };
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Chooses rsync (default) or scp, reporting whether we fell back and a note to
/// show the user.
pub(crate) fn choose_tool(
    use_scp: bool,
    local_rsync_present: bool,
) -> (Tool, bool, Option<String>) {
    if use_scp {
        (Tool::Scp, false, None)
    } else if !local_rsync_present {
        (
            Tool::Scp,
            true,
            Some("rsync not found locally; using scp".to_string()),
        )
    } else {
        (Tool::Rsync, false, None)
    }
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
    let remote = format!("{}:{}", ctx.host, remote_spec);
    let local = local_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    match direction {
        Direction::Download => format!("{remote} -> {local}"),
        Direction::Upload => format!("{local} -> {remote}"),
    }
}

/// Spawns the transfer command and relays its stdout/stderr to the remote as
/// `Log` frames, returning the child's exit code. Argv only — never a shell.
async fn spawn_and_relay(stream: &mut impl DuplexStream, command: TransferCommand) -> Result<i32> {
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", command.program))?;

    let mut stdout = child
        .stdout
        .take()
        .context("child stdout was not captured")?;
    let mut stderr = child
        .stderr
        .take()
        .context("child stderr was not captured")?;

    let mut out_buf = vec![0u8; 16 * 1024];
    let mut err_buf = vec![0u8; 16 * 1024];
    let mut out_open = true;
    let mut err_open = true;

    while out_open || err_open {
        tokio::select! {
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

    let status = child.wait().await.context("waiting for transfer child")?;
    Ok(status.code().unwrap_or(-1))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(host: &str, ssh_options: Vec<&str>, control_path: Option<&str>) -> SessionContext {
        SessionContext {
            host: host.to_string(),
            ssh_options: ssh_options.into_iter().map(String::from).collect(),
            local_dir: PathBuf::from("/home/u/proj"),
            control_path: control_path.map(PathBuf::from),
        }
    }

    fn plan(direction: Direction, remote: &str, locals: &[&str], force: bool) -> TransferPlan {
        TransferPlan {
            direction,
            remote_spec: remote.to_string(),
            local_paths: locals.iter().map(PathBuf::from).collect(),
            force,
            dry_run: false,
        }
    }

    #[test]
    fn rsync_download_argv_uses_dashdash_and_control_master() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(
            Direction::Download,
            "/abs/logs/*.log",
            &["/home/u/proj"],
            false,
        );
        let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
        assert_eq!(cmd.program, "rsync");
        assert!(cmd.args.contains(&"--".to_string()));
        assert!(cmd.args.contains(&"-a".to_string()));
        assert!(cmd.args.contains(&"--ignore-existing".to_string()));
        // -e string is built only from ctx (control master), never the wire.
        let e_idx = cmd.args.iter().position(|a| a == "-e").unwrap();
        assert_eq!(
            cmd.args[e_idx + 1],
            "ssh -o ControlPath=/run/ctl.sock -o ControlMaster=no"
        );
        // remote operand is host:spec, after the -- separator.
        let sep = cmd.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(cmd.args[sep + 1], "dev:/abs/logs/*.log");
        assert_eq!(cmd.args[sep + 2], "/home/u/proj");
    }

    #[test]
    fn rsync_upload_puts_local_sources_before_remote_dest() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(
            Direction::Upload,
            "/remote/dir",
            &["/home/u/proj/a.txt", "/home/u/proj/b.txt"],
            true,
        );
        let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
        assert!(!cmd.args.contains(&"--ignore-existing".to_string())); // force set
        let sep = cmd.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(cmd.args[sep + 1], "/home/u/proj/a.txt");
        assert_eq!(cmd.args[sep + 2], "/home/u/proj/b.txt");
        assert_eq!(cmd.args[sep + 3], "dev:/remote/dir");
    }

    #[test]
    fn scp_download_argv_shape() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(Direction::Download, "/abs/file", &["/home/u/proj"], false);
        let cmd = build_transfer_argv(Tool::Scp, &ctx, &plan).unwrap();
        assert_eq!(cmd.program, "scp");
        assert!(cmd.args.contains(&"-r".to_string()));
        let sep = cmd.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(cmd.args[sep + 1], "dev:/abs/file");
        assert_eq!(cmd.args[sep + 2], "/home/u/proj");
    }

    #[test]
    fn hostile_remote_spec_never_becomes_an_option() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(
            Direction::Download,
            "-oProxyCommand=evil",
            &["/home/u/proj"],
            false,
        );
        let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
        // It only ever appears glued behind the host and after --.
        assert!(cmd.args.contains(&"dev:-oProxyCommand=evil".to_string()));
        assert!(!cmd.args.iter().any(|a| a == "-oProxyCommand=evil"));
    }

    #[test]
    fn remote_spec_with_newline_is_rejected() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(Direction::Download, "a\nb", &["/home/u/proj"], false);
        assert!(build_transfer_argv(Tool::Rsync, &ctx, &plan).is_err());
    }

    #[test]
    fn local_operand_leading_dash_is_dot_slashed() {
        let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
        let plan = plan(Direction::Upload, "/remote", &["-rf"], true);
        let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
        assert!(cmd.args.contains(&"./-rf".to_string()));
        assert!(!cmd.args.iter().any(|a| a == "-rf"));
    }

    #[test]
    fn rsync_rsh_rejects_whitespace_options() {
        let ctx = ctx_with("dev", vec!["-oProxyCommand=ssh jump nc %h %p"], None);
        assert!(rsync_rsh(&ctx).is_err());
    }

    #[test]
    fn rsync_rsh_none_for_bare_alias() {
        let ctx = ctx_with("dev", vec![], None);
        assert_eq!(rsync_rsh(&ctx).unwrap(), None);
    }

    #[test]
    fn choose_tool_prefers_rsync_but_falls_back() {
        assert_eq!(choose_tool(false, true).0, Tool::Rsync);
        assert_eq!(choose_tool(true, true).0, Tool::Scp);
        let (tool, fell_back, note) = choose_tool(false, false);
        assert_eq!(tool, Tool::Scp);
        assert!(fell_back);
        assert!(note.is_some());
    }

    #[test]
    fn expand_hint_is_tilde_only_and_anchors_relative() {
        let base = Path::new("/home/u/proj");
        assert_eq!(expand_hint(base, "sub/file"), base.join("sub/file"));
        assert_eq!(expand_hint(base, "/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn detects_glob_metacharacters() {
        assert!(has_glob_metacharacters("*.log"));
        assert!(has_glob_metacharacters("a[0-9]"));
        assert!(!has_glob_metacharacters("plain.txt"));
    }

    #[test]
    fn resolve_local_sources_expands_glob_and_errors_on_no_match() {
        let dir = std::env::temp_dir().join(format!("shine-glob-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::write(dir.join("b.txt"), b"b").unwrap();
        std::fs::write(dir.join("c.log"), b"c").unwrap();
        let ctx = SessionContext {
            host: "dev".to_string(),
            ssh_options: vec![],
            local_dir: dir.clone(),
            control_path: None,
        };

        let mut matched = resolve_local_sources(&ctx, Some("*.txt")).unwrap();
        matched.sort();
        assert_eq!(matched, vec![dir.join("a.txt"), dir.join("b.txt")]);

        // A non-glob source is returned verbatim (resolved), even if missing —
        // rsync/scp reports a missing single source itself.
        assert_eq!(
            resolve_local_sources(&ctx, Some("c.log")).unwrap(),
            vec![dir.join("c.log")]
        );

        assert!(resolve_local_sources(&ctx, Some("*.missing")).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
