//! `shine ssh`: wraps the system `ssh` binary to establish an interactive
//! session that also carries a session-scoped file-transfer channel back to
//! the local machine (see docs/ssh-local-transfer-prd.md).
//!
//! Architecture, confirmed against a real host via `scripts/spike-ssh-forward.sh`:
//! - We prepend our own `-R <remote-sock>:<local-forward-target>` to the
//!   user's ssh args (safe: ssh options may appear in any order before the
//!   destination).
//! - We replace the remote command with a wrapper that sets
//!   `SHINE_SSH_SESSION`/`SHINE_SSH_TOKEN`/`SHINE_SSH_REMOTE_SOCK` via `env`
//!   (not `SetEnv`/`SendEnv`, which most sshd configs don't accept), then
//!   `exec`s either the user's original remote command or their login shell.
//!   Explicit `--with`/`--with-secret` values join that process environment.
//! - sshd does NOT clean up the forwarded remote socket file on disconnect
//!   (confirmed by the spike), so the wrapper registers its own `trap ...
//!   EXIT` to remove it.
//!
//! The default remote mode is POSIX: it uses a Unix socket and a POSIX shell
//! wrapper regardless of the *local* platform. `--remote-shell windows` is
//! an explicit environment-forwarding-only mode: it sends a Base64-encoded
//! PowerShell bootstrap, preferring PowerShell 7 (`pwsh.exe`) and falling
//! back to Windows PowerShell (`powershell.exe`), and deliberately creates no
//! transfer listener or `-R` forward. Locally, the POSIX path's
//! `bind_local_listener` uses a Unix socket on macOS/Linux, or loopback TCP on
//! Windows.

mod arguments;
mod command;
use arguments::{
    ParsedSshArgs, VALUE_OPTION_LETTERS, build_ssh_invocation_args,
    build_windows_ssh_invocation_args, split_ssh_args,
};
use command::*;
mod agent;
mod broker;
// Drives the real agent over an in-process Unix socket pair, so it is
// unix-only (Windows is the local side only and has no `UnixListener`).
#[cfg(all(test, unix))]
mod integration_tests;
mod protocol;
mod session_context;
// `remote_client` dials the forwarded socket via a Unix stream: it only
// ever runs on the *remote* end of a session, which is always assumed
// Linux/macOS (see module docs), so it is unconditionally unix-only —
// unlike `agent`, which must compile on Windows too since Windows is
// supported as the *local* side.
#[cfg(unix)]
mod remote_client;

use crate::env::RESERVED_REMOTE_ENV;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::commands::RemoteShell;
use crate::config::Config;
use crate::env::{EnvConfig, parse_env_specs, secret_key};
use crate::secret;
use crate::theme;

/// Grace period given to still-running per-connection transfer tasks to
/// notice the (by now closed) `ssh` tunnel and finish their own cleanup
/// before the session directory is removed. See
/// `agent::drain_connection_tasks`.
const CONNECTION_DRAIN_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(not(unix))]
const WINDOWS_REMOTE_UNSUPPORTED: &str = "`shine local` commands require this machine to be the \
    remote (Linux/macOS) side of a `shine ssh` session; Windows is currently supported as the \
    local side only";

#[cfg(unix)]
pub async fn handle_local_download(
    remote_source: &str,
    local_destination: Option<&str>,
    force: bool,
    dry_run: bool,
    use_scp: bool,
) -> Result<()> {
    remote_client::handle_download(remote_source, local_destination, force, dry_run, use_scp).await
}

#[cfg(not(unix))]
pub async fn handle_local_download(
    _remote_source: &str,
    _local_destination: Option<&str>,
    _force: bool,
    _dry_run: bool,
    _use_scp: bool,
) -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(unix)]
pub async fn handle_local_upload(
    local_source: &str,
    remote_destination: Option<&str>,
    force: bool,
    dry_run: bool,
    use_scp: bool,
) -> Result<()> {
    remote_client::handle_upload(local_source, remote_destination, force, dry_run, use_scp).await
}

#[cfg(not(unix))]
pub async fn handle_local_upload(
    _local_source: &str,
    _remote_destination: Option<&str>,
    _force: bool,
    _dry_run: bool,
    _use_scp: bool,
) -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(unix)]
pub async fn handle_local_status() -> Result<()> {
    remote_client::handle_status().await
}

#[cfg(unix)]
pub async fn request_direct_secrets(
    specs: &[String],
    argv: &[String],
) -> Result<BTreeMap<String, String>> {
    remote_client::request_direct_secrets(specs, argv).await
}

#[cfg(not(unix))]
pub async fn request_direct_secrets(
    _specs: &[String],
    _argv: &[String],
) -> Result<BTreeMap<String, String>> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(unix)]
pub async fn request_workspace_secrets(
    snapshot: crate::env::broker::WorkspaceSnapshot,
    argv: &[String],
) -> Result<BTreeMap<String, String>> {
    remote_client::request_workspace_secrets(snapshot, argv).await
}

#[cfg(unix)]
pub fn broker_session_available() -> bool {
    remote_client::session_available()
}

#[cfg(not(unix))]
pub fn broker_session_available() -> bool {
    false
}

#[cfg(unix)]
pub async fn describe_broker_workspace(
    snapshot: crate::env::broker::WorkspaceSnapshot,
    release: &[String],
    argv: &[String],
) -> Result<String> {
    remote_client::describe_workspace(snapshot, release, argv).await
}

#[cfg(not(unix))]
pub async fn describe_broker_workspace(
    _snapshot: crate::env::broker::WorkspaceSnapshot,
    _release: &[String],
    _argv: &[String],
) -> Result<String> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(not(unix))]
pub async fn request_workspace_secrets(
    _snapshot: crate::env::broker::WorkspaceSnapshot,
    _argv: &[String],
) -> Result<BTreeMap<String, String>> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(not(unix))]
pub async fn handle_local_status() -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

/// Owns session resources from the first setup step, including early errors.
struct SshSession {
    directory: PathBuf,
    listener_task: Option<tokio::task::JoinHandle<()>>,
    connections: agent::ConnectionTasks,
}

impl SshSession {
    async fn create(directory: PathBuf) -> Result<Self> {
        tokio::fs::create_dir_all(directory.parent().context("SSH session has no parent")?).await?;
        // Establish exclusive ownership before arming cleanup, even if a
        // pre-existing entry happens to use the generated session identifier.
        tokio::fs::create_dir(&directory).await?;
        Ok(Self {
            directory,
            listener_task: None,
            connections: agent::new_connection_tasks(),
        })
    }

    async fn shutdown(&mut self) {
        if let Some(listener) = self.listener_task.take() {
            listener.abort();
            let _ = listener.await;
        }
        agent::drain_connection_tasks(&self.connections, CONNECTION_DRAIN_GRACE_PERIOD).await;
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        if let Some(listener) = &self.listener_task {
            listener.abort();
        }
        if let Ok(mut connections) = self.connections.try_lock() {
            connections.abort_all();
        }
        if let Err(error) = std::fs::remove_dir_all(&self.directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "Warning: could not clean SSH session {}: {error}",
                self.directory.display()
            );
        }
    }
}

#[allow(clippy::too_many_arguments)] // Top-level handler mirrors the independently meaningful CLI switches.
pub async fn handle_ssh(
    config: &Config,
    remote_shell: RemoteShell,
    with: &[String],
    with_secret: &[String],
    secret_broker: bool,
    secret_broker_policy: &[PathBuf],
    allow_secret: &[String],
    trust_remote_session: bool,
    secret_broker_inspect: bool,
    secret_broker_enroll: bool,
    trust_remote_metadata: bool,
    secret_broker_update_policy: Option<&str>,
    args: &[String],
) -> Result<()> {
    let ParsedSshArgs {
        options: ssh_options,
        host,
        remote_command,
    } = split_ssh_args(args)?;
    let forwarded_env = resolve_forwarded_env(config, with, with_secret).await?;

    // Windows OpenSSH executes remote commands through cmd.exe by default.
    // Do not create the POSIX-only transfer channel there; the encoded
    // PowerShell command has no user-controlled syntax in cmd.exe.
    if remote_shell == RemoteShell::Windows {
        if secret_broker
            || !secret_broker_policy.is_empty()
            || !allow_secret.is_empty()
            || trust_remote_session
            || secret_broker_inspect
            || secret_broker_enroll
            || secret_broker_update_policy.is_some()
        {
            bail!("SSH secret broker requires the POSIX remote shell mode");
        }
        let session_id = uuid::Uuid::new_v4().to_string();
        let local_theme = theme::resolve_local_terminal_theme_for_injection();
        let wrapped_command = build_windows_wrapped_remote_command(
            &session_id,
            local_theme.map(theme::Theme::as_str),
            &forwarded_env,
            &remote_command,
        )?;
        let mut cmd = tokio::process::Command::new("ssh");
        cmd.args(build_windows_ssh_invocation_args(
            &ssh_options,
            &host,
            &wrapped_command,
        ));
        return finish_ssh_status(run_ssh_with_ctrl_c(&mut cmd).await?);
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let broker_session = broker::BrokerSession::prepare(
        config,
        &host,
        secret_broker,
        secret_broker_policy,
        allow_secret,
        trust_remote_session,
        secret_broker_inspect,
        secret_broker_enroll,
        trust_remote_metadata,
        secret_broker_update_policy,
    )
    .await?;

    let session_dir = config.shine_dir().join("run").join("ssh").join(&session_id);
    let mut session = SshSession::create(session_dir.clone())
        .await
        .with_context(|| format!("creating {}", session_dir.display()))?;
    // The remote host is always assumed Linux/macOS (see module docs), so
    // its socket is always a Unix socket regardless of the local platform.
    let remote_sock = format!("/tmp/.shine-ssh-{session_id}.sock");

    let (listener, local_forward_target) = bind_local_listener(&session_dir).await?;
    let session_local_dir = std::env::current_dir().context("reading current directory")?;

    // Reuse the interactive connection as a control master so the rsync/scp
    // child reconnects over it with no second authentication (ADR 0011). Skip
    // if the user already configured their own multiplexing, so we don't fight
    // their settings.
    let control_options = if session_context::user_set_control_options(&ssh_options) {
        None
    } else {
        Some(session_dir.join("ctl.sock"))
    };

    let context = std::sync::Arc::new(session_context::SessionContext {
        host: host.clone(),
        ssh_options: ssh_options.clone(),
        local_dir: session_local_dir.clone(),
        control_path: control_options.clone(),
    });
    context.save(&session_dir).await?;

    session.listener_task = Some(tokio::spawn(listener.serve(
        token.clone(),
        context.clone(),
        broker_session.clone(),
        session.connections.clone(),
    )));

    // Query the *local* terminal — same-host, sub-millisecond round trip,
    // no fragmentation risk unlike a remote OSC query (PRD §2.2/§6.1) — so
    // the remote login shell never has to guess at its own theme.
    let local_theme = theme::resolve_local_terminal_theme_for_injection();
    let wrapped_command = build_wrapped_remote_command(
        &session_id,
        &token,
        &remote_sock,
        local_theme.map(theme::Theme::as_str),
        &forwarded_env,
        &remote_command,
    );

    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(build_ssh_invocation_args(
        &ssh_options,
        &remote_sock,
        &local_forward_target,
        control_options.as_deref(),
        &host,
        &wrapped_command,
    ));

    // Racing against ctrl_c() (rather than just awaiting cmd.status()) is
    // what makes the cleanup below actually run on Ctrl-C: installing this
    // listener overrides SIGINT's default disposition for the process, so a
    // Ctrl-C no longer kills us before we get a chance to clean up. The ssh
    // child is in the same foreground process group and receives SIGINT
    // independently; we still await its exit so we don't race it.
    let status = run_ssh_with_ctrl_c_broker(&mut cmd, broker_session.as_deref()).await;

    // Stop accepting new connections, then give any still-running transfer
    // a bounded chance to notice the tunnel is gone and run its own
    // cleanup before we remove the session directory out from under it.
    session.shutdown().await;
    drop(session);

    finish_ssh_status(status?)
}

async fn run_ssh_with_ctrl_c_broker(
    cmd: &mut tokio::process::Command,
    broker: Option<&broker::BrokerSession>,
) -> Result<std::process::ExitStatus> {
    let mut child = cmd.spawn().context("failed to start ssh")?;
    if let Some(broker) = broker {
        broker.set_ssh_pid(child.id());
    }
    let mut wait = std::pin::pin!(child.wait());
    let result = tokio::select! {
        status = &mut wait => status,
        _ = tokio::signal::ctrl_c() => wait.await,
    }
    .context("failed to run ssh");
    if let Some(broker) = broker {
        broker.set_ssh_pid(None);
    }
    result
}

async fn run_ssh_with_ctrl_c(
    cmd: &mut tokio::process::Command,
) -> Result<std::process::ExitStatus> {
    let mut ssh_run = std::pin::pin!(cmd.status());
    tokio::select! {
        status = &mut ssh_run => status,
        _ = tokio::signal::ctrl_c() => ssh_run.await,
    }
    .context("failed to run ssh")
}

fn finish_ssh_status(status: std::process::ExitStatus) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::exit(128 + status.signal().unwrap_or(1));
    }
    #[cfg(not(unix))]
    std::process::exit(1);
}

/// Resolves only explicitly selected config values. Plaintext selection is
/// deliberately exact: unlike `shine env run --with`, it never falls through
/// to `<KEY>_SECRET`. Sending decrypted material to another host requires the
/// visibly distinct `--with-secret` opt-in.
async fn resolve_forwarded_env(
    config: &Config,
    with: &[String],
    with_secret: &[String],
) -> Result<BTreeMap<String, String>> {
    let plain_specs = parse_env_specs(with)?;
    let secret_specs = parse_env_specs(with_secret)?;
    let env = EnvConfig::from_config(config);
    let mut targets = BTreeSet::new();
    let mut resolved = BTreeMap::new();

    for spec in plain_specs {
        validate_forward_target(&spec.target, &mut targets)?;
        if spec.source.ends_with("_SECRET") {
            bail!(
                "--with does not inject secret storage key {}; use --with-secret with the base key instead",
                spec.source
            );
        }
        let value = env.get(&spec.source).with_context(|| {
            let encrypted = secret_key(&spec.source);
            if env.get(&encrypted).is_some() {
                format!(
                    "{} is stored as {encrypted}; use --with-secret {} to decrypt and inject it",
                    spec.source, spec.source
                )
            } else {
                format!("{} is not set in the active config [env]", spec.source)
            }
        })?;
        resolved.insert(spec.target, value.to_string());
    }

    for spec in secret_specs {
        validate_forward_target(&spec.target, &mut targets)?;
        if spec.source.ends_with("_SECRET") {
            bail!(
                "--with-secret expects a base key without the _SECRET suffix: {}",
                spec.source
            );
        }
        let encrypted = secret_key(&spec.source);
        let ciphertext = env
            .get(&encrypted)
            .with_context(|| format!("{encrypted} is not set in the active config [env]"))?;
        let value = secret::decrypt_with_config(ciphertext, config)
            .await
            .with_context(|| format!("decrypting {encrypted}"))?;
        resolved.insert(spec.target, value);
    }

    Ok(resolved)
}

fn validate_forward_target(target: &str, targets: &mut BTreeSet<String>) -> Result<()> {
    if RESERVED_REMOTE_ENV.contains(&target) {
        bail!("cannot override shine-managed SSH variable {target}");
    }
    if !targets.insert(target.to_string()) {
        bail!("duplicate target variable: {target}");
    }
    Ok(())
}

/// Binds the local end of the session's transfer channel and returns it
/// together with the target to embed in `ssh`'s `-R <remote-sock>:<target>`
/// argument.
#[cfg(unix)]
async fn bind_local_listener(
    session_dir: &std::path::Path,
) -> Result<(agent::LocalListener, String)> {
    let local_sock = session_dir.join("local.sock");
    let listener = tokio::net::UnixListener::bind(&local_sock)
        .with_context(|| format!("binding local transfer socket {}", local_sock.display()))?;
    Ok((
        agent::LocalListener::Unix(listener),
        local_sock.display().to_string(),
    ))
}

/// Windows lacks the mature, well-tested Unix-domain-socket support that
/// macOS/Linux have, so the local end uses a loopback TCP socket instead;
/// `ssh -R` supports mixing this with the remote's Unix-socket endpoint
/// (verified via `scripts/spike-ssh-forward-windows.ps1`).
#[cfg(windows)]
async fn bind_local_listener(
    _session_dir: &std::path::Path,
) -> Result<(agent::LocalListener, String)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding local transfer TCP listener")?;
    let port = listener
        .local_addr()
        .context("reading local TCP listener port")?
        .port();
    Ok((
        agent::LocalListener::Tcp(listener),
        format!("127.0.0.1:{port}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_creation_preserves_an_existing_directory() {
        let directory = crate::test_support::make_temp_dir("shine-session-collision").await;
        let keep = directory.join("keep");
        tokio::fs::write(&keep, b"user contents").await.unwrap();
        assert!(SshSession::create(directory.clone()).await.is_err());
        assert_eq!(tokio::fs::read(keep).await.unwrap(), b"user contents");
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[test]
    fn plain_host_with_no_options_or_command() {
        let ParsedSshArgs {
            options,
            host,
            remote_command: command,
        } = split_ssh_args(&["dev".to_string()]).unwrap();
        assert!(options.is_empty());
        assert_eq!(host, "dev");
        assert!(command.is_empty());
    }

    #[test]
    fn host_followed_by_a_remote_command() {
        let args = vec!["dev".to_string(), "ls".to_string(), "-la".to_string()];
        let ParsedSshArgs {
            options,
            host,
            remote_command: command,
        } = split_ssh_args(&args).unwrap();
        assert!(options.is_empty());
        assert_eq!(host, "dev");
        assert_eq!(command, vec!["ls", "-la"]);
    }

    #[test]
    fn value_option_with_separate_token() {
        let args = vec!["-p".to_string(), "2222".to_string(), "dev".to_string()];
        let ParsedSshArgs {
            options,
            host,
            remote_command: command,
        } = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-p", "2222"]);
        assert_eq!(host, "dev");
        assert!(command.is_empty());
    }

    #[test]
    fn value_option_with_attached_value() {
        let args = vec!["-p2222".to_string(), "dev".to_string()];
        let ParsedSshArgs {
            options,
            host,
            remote_command: _command,
        } = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-p2222"]);
        assert_eq!(host, "dev");
    }

    #[test]
    fn repeated_o_option() {
        let args = vec![
            "-o".to_string(),
            "ProxyJump=bastion".to_string(),
            "dev".to_string(),
        ];
        let ParsedSshArgs {
            options,
            host,
            remote_command: _command,
        } = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-o", "ProxyJump=bastion"]);
        assert_eq!(host, "dev");
    }

    #[test]
    fn bundled_boolean_flags_consume_no_value() {
        let args = vec!["-vvv".to_string(), "dev".to_string()];
        let ParsedSshArgs {
            options,
            host,
            remote_command: _command,
        } = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-vvv"]);
        assert_eq!(host, "dev");
    }

    #[test]
    fn explicit_double_dash_separator() {
        let args = vec!["--".to_string(), "dev".to_string(), "ls".to_string()];
        let ParsedSshArgs {
            options,
            host,
            remote_command: command,
        } = split_ssh_args(&args).unwrap();
        assert!(options.is_empty());
        assert_eq!(host, "dev");
        assert_eq!(command, vec!["ls"]);
    }

    #[test]
    fn no_destination_is_an_error() {
        assert!(split_ssh_args(&[]).is_err());
    }

    #[test]
    fn dangling_value_option_is_an_error() {
        assert!(split_ssh_args(&["-p".to_string()]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn wrapped_command_round_trips_through_a_real_shell() {
        // Exercises the full nested-quoting composition end to end: run the
        // wrapped command through `sh -c` and check the remote command's
        // stdout, rather than reasoning about escaping by hand.
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/shine-ssh-mod-test-sid.sock",
            None,
            &BTreeMap::new(),
            &["echo".to_string(), "it's a test".to_string()],
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&wrapped)
            .output()
            .expect("failed to run sh");
        assert!(output.status.success(), "stderr: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "it's a test"
        );
    }

    #[test]
    fn ssh_invocation_args_keep_user_options_verbatim_and_ahead_of_our_own() {
        let args = vec!["-J".to_string(), "bastion".to_string()];
        let ParsedSshArgs {
            options: parsed_options,
            host,
            remote_command: _command,
        } = split_ssh_args(&[args.clone(), vec!["dev".to_string()]].concat()).unwrap();

        let invocation = build_ssh_invocation_args(
            &parsed_options,
            "/tmp/.shine-ssh-sid.sock",
            "/tmp/shine-ssh-sid/local.sock",
            None,
            &host,
            "wrapped-command",
        );

        assert_eq!(
            invocation,
            vec![
                "-J",
                "bastion",
                "-t",
                "-R",
                "/tmp/.shine-ssh-sid.sock:/tmp/shine-ssh-sid/local.sock",
                "dev",
                "wrapped-command",
            ]
        );
    }

    #[test]
    fn windows_ssh_invocation_has_no_transfer_or_posix_wrapper() {
        let invocation = build_windows_ssh_invocation_args(
            &["-p".to_string(), "2222".to_string()],
            "windows-host",
            "powershell.exe -NoProfile -EncodedCommand QQ==",
        );

        assert_eq!(
            invocation,
            vec![
                "-p",
                "2222",
                "-t",
                "windows-host",
                "powershell.exe -NoProfile -EncodedCommand QQ==",
            ]
        );
        assert!(!invocation.iter().any(|arg| arg == "-R"));
        assert!(!invocation.iter().any(|arg| arg.contains("env ")));
        assert!(!invocation.iter().any(|arg| arg.contains("sh -c")));
    }

    #[test]
    fn ssh_invocation_args_inject_control_master_when_control_path_given() {
        let ParsedSshArgs {
            options: parsed_options,
            host,
            remote_command: _command,
        } = split_ssh_args(&["dev".to_string()]).unwrap();
        let invocation = build_ssh_invocation_args(
            &parsed_options,
            "/tmp/.shine-ssh-sid.sock",
            "/tmp/shine-ssh-sid/local.sock",
            Some(std::path::Path::new("/tmp/shine-ssh-sid/ctl.sock")),
            &host,
            "wrapped-command",
        );

        assert_eq!(
            invocation,
            vec![
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=/tmp/shine-ssh-sid/ctl.sock",
                "-o",
                "ControlPersist=60",
                "-t",
                "-R",
                "/tmp/.shine-ssh-sid.sock:/tmp/shine-ssh-sid/local.sock",
                "dev",
                "wrapped-command",
            ]
        );
    }

    #[test]
    fn ssh_invocation_args_preserve_repeated_o_options_in_order() {
        let args = vec![
            "-o".to_string(),
            "ProxyJump=bastion".to_string(),
            "-o".to_string(),
            "ServerAliveInterval=30".to_string(),
            "dev".to_string(),
            "ls".to_string(),
            "-la".to_string(),
        ];
        let ParsedSshArgs {
            options: parsed_options,
            host,
            remote_command: command,
        } = split_ssh_args(&args).unwrap();
        assert_eq!(command, vec!["ls", "-la"]);

        let invocation = build_ssh_invocation_args(
            &parsed_options,
            "/tmp/.shine-ssh-sid.sock",
            "/tmp/shine-ssh-sid/local.sock",
            None,
            &host,
            "wrapped-command",
        );

        // The user's repeated -o options must appear verbatim, in order, and
        // ahead of our own -t/-R/host/command — never reordered or merged.
        assert_eq!(
            invocation,
            vec![
                "-o",
                "ProxyJump=bastion",
                "-o",
                "ServerAliveInterval=30",
                "-t",
                "-R",
                "/tmp/.shine-ssh-sid.sock:/tmp/shine-ssh-sid/local.sock",
                "dev",
                "wrapped-command",
            ]
        );
    }

    #[test]
    fn wrapped_command_defaults_to_login_shell() {
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/.shine-ssh-sid.sock",
            None,
            &BTreeMap::new(),
            &[],
        );
        assert!(wrapped.contains(r#"exec "$SHELL" -l"#));
        assert!(wrapped.contains("trap \"rm -f $SHINE_SSH_REMOTE_SOCK\" EXIT"));
    }

    #[test]
    fn wrapped_command_omits_theme_var_when_none() {
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/.shine-ssh-sid.sock",
            None,
            &BTreeMap::new(),
            &[],
        );
        assert!(!wrapped.contains("SHINE_TERMINAL_THEME"));
    }

    #[test]
    fn wrapped_command_injects_quoted_theme_var_when_present() {
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/.shine-ssh-sid.sock",
            Some("dark"),
            &BTreeMap::new(),
            &[],
        );
        assert!(wrapped.contains("SHINE_TERMINAL_THEME='dark'"));
        // Must appear inside the `env ...` prefix, before the `sh -c` handoff.
        assert!(
            wrapped.find("SHINE_TERMINAL_THEME").unwrap() < wrapped.find("sh -c").unwrap(),
            "theme var must be part of the env prefix: {wrapped}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wrapped_command_theme_injection_round_trips_through_a_real_shell() {
        // Same rationale as wrapped_command_round_trips_through_a_real_shell:
        // verify the quoting composition by actually running it, rather than
        // reasoning about escaping by hand.
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/shine-ssh-mod-test-theme-sid.sock",
            Some("dark"),
            &BTreeMap::new(),
            &["printenv".to_string(), "SHINE_TERMINAL_THEME".to_string()],
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&wrapped)
            .output()
            .expect("failed to run sh");
        assert!(output.status.success(), "stderr: {:?}", output.stderr);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), "dark");
    }

    #[cfg(unix)]
    #[test]
    fn wrapped_command_forwarded_env_round_trips_special_characters() {
        let forwarded = BTreeMap::from([(
            "REMOTE_VALUE".to_string(),
            "space ' quote $dollar\nand newline".to_string(),
        )]);
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/shine-ssh-mod-test-env-sid.sock",
            None,
            &forwarded,
            &["printenv".to_string(), "REMOTE_VALUE".to_string()],
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&wrapped)
            .output()
            .expect("failed to run sh");
        assert!(output.status.success(), "stderr: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "space ' quote $dollar\nand newline\n"
        );
    }

    #[test]
    fn windows_wrapped_command_decodes_special_values_and_command_argv() {
        let forwarded = BTreeMap::from([(
            "REMOTE_VALUE".to_string(),
            "space ' quote \" $dollar & amp % percent ! bang\nand newline".to_string(),
        )]);
        let wrapped = build_windows_wrapped_remote_command(
            "sid",
            Some("dark"),
            &forwarded,
            &[
                "C:\\Program Files\\tool.exe".to_string(),
                "one ' $ & % !".to_string(),
            ],
        )
        .unwrap();

        assert!(wrapped.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!wrapped.contains("REMOTE_VALUE"));
        assert!(!wrapped.contains("$dollar"));
        let bootstrap = decode_powershell_script(wrapped.rsplit_once(' ').unwrap().1);
        let encoded = bootstrap
            .split(" -EncodedCommand '")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        let script = decode_powershell_script(encoded);
        assert!(!bootstrap.contains("REMOTE_VALUE"));
        assert!(!bootstrap.contains("$dollar"));
        assert!(bootstrap.contains("Get-Command pwsh.exe"));
        assert!(bootstrap.contains("else { 'powershell.exe' }"));
        assert!(bootstrap.contains("& $shell -NoProfile -EncodedCommand"));
        assert!(bootstrap.contains("exit $code"));
        assert!(script.contains("$env:SHINE_SSH_SESSION = 'sid'"));
        assert!(script.contains("$env:SHINE_TERMINAL_THEME = 'dark'"));
        assert!(script.contains(
            "$env:REMOTE_VALUE = 'space '' quote \" $dollar & amp % percent ! bang\nand newline'"
        ));
        assert!(script.contains("& 'C:\\Program Files\\tool.exe' 'one '' $ & % !'"));
        assert!(script.contains("exit $LASTEXITCODE"));
    }

    fn decode_powershell_script(encoded: &str) -> String {
        let bytes = BASE64.decode(encoded).unwrap();
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn windows_wrapped_command_uses_no_exit_for_interactive_session() {
        let wrapped =
            build_windows_wrapped_remote_command("sid", None, &BTreeMap::new(), &[]).unwrap();
        let bootstrap = decode_powershell_script(wrapped.rsplit_once(' ').unwrap().1);
        assert!(bootstrap.contains("& $shell -NoExit -EncodedCommand "));
        assert!(!bootstrap.contains("& $shell -NoProfile"));
    }

    #[test]
    fn windows_wrapped_command_selects_shell_before_running_payload() {
        let wrapped = build_windows_wrapped_remote_command(
            "sid",
            None,
            &BTreeMap::new(),
            &["exit".to_string(), "7".to_string()],
        )
        .unwrap();

        assert!(wrapped.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        let bootstrap = decode_powershell_script(wrapped.rsplit_once(' ').unwrap().1);
        assert!(bootstrap.contains("Get-Command pwsh.exe -CommandType Application"));
        assert!(bootstrap.contains("$pwsh.Source"));
        assert!(bootstrap.contains("else { 'powershell.exe' }"));
        assert_eq!(bootstrap.matches("& $shell").count(), 1);
        assert!(!wrapped.contains("&&"));
        assert!(!wrapped.contains("||"));
    }

    #[test]
    fn windows_wrapped_command_rejects_nul_values() {
        let forwarded = BTreeMap::from([("REMOTE_VALUE".to_string(), "bad\0value".to_string())]);
        let error = build_windows_wrapped_remote_command("sid", None, &forwarded, &[]).unwrap_err();
        assert!(error.to_string().contains("NUL"));
    }

    #[tokio::test]
    async fn forwarded_plain_env_uses_exact_key_and_alias() {
        let dir = std::env::temp_dir().join(format!("shine-ssh-env-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.env.insert("LOCAL_NAME".into(), "local value".into());
        config
            .env
            .insert("LOCAL_NAME_SECRET".into(), "encrypted value".into());

        let resolved = resolve_forwarded_env(&config, &["LOCAL_NAME=REMOTE_NAME".to_string()], &[])
            .await
            .unwrap();

        assert_eq!(
            resolved.get("REMOTE_NAME").map(String::as_str),
            Some("local value")
        );
    }

    #[tokio::test]
    async fn forwarded_plain_env_requires_explicit_secret_opt_in() {
        let dir = std::env::temp_dir().join(format!("shine-ssh-env-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config
            .env
            .insert("API_TOKEN_SECRET".into(), "ciphertext".into());

        let error = resolve_forwarded_env(&config, &["API_TOKEN".to_string()], &[])
            .await
            .unwrap_err();

        assert!(error.to_string().contains("use --with-secret API_TOKEN"));
    }

    #[tokio::test]
    async fn forwarded_secret_requires_base_key_and_encrypted_storage() {
        let dir = std::env::temp_dir().join(format!("shine-ssh-env-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.env.insert("API_TOKEN".into(), "plaintext".into());
        config
            .env
            .insert("OTHER_SECRET".into(), "ciphertext".into());

        let plaintext_only = resolve_forwarded_env(&config, &[], &["API_TOKEN".to_string()])
            .await
            .unwrap_err();
        assert!(
            plaintext_only
                .to_string()
                .contains("API_TOKEN_SECRET is not set")
        );

        let suffixed = resolve_forwarded_env(&config, &[], &["OTHER_SECRET".to_string()])
            .await
            .unwrap_err();
        assert!(
            suffixed
                .to_string()
                .contains("expects a base key without the _SECRET suffix")
        );
    }

    #[tokio::test]
    async fn forwarded_env_rejects_duplicate_and_reserved_targets() {
        let dir = std::env::temp_dir().join(format!("shine-ssh-env-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.env.insert("ONE".into(), "1".into());
        config.env.insert("TWO".into(), "2".into());

        let duplicate = resolve_forwarded_env(
            &config,
            &["ONE=REMOTE".to_string(), "TWO=REMOTE".to_string()],
            &[],
        )
        .await
        .unwrap_err();
        assert!(
            duplicate
                .to_string()
                .contains("duplicate target variable: REMOTE")
        );

        let reserved = resolve_forwarded_env(&config, &["ONE=SHINE_SSH_TOKEN".to_string()], &[])
            .await
            .unwrap_err();
        assert!(
            reserved
                .to_string()
                .contains("cannot override shine-managed SSH variable SHINE_SSH_TOKEN")
        );
    }
}
