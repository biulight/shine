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
//! - sshd does NOT clean up the forwarded remote socket file on disconnect
//!   (confirmed by the spike), so the wrapper registers its own `trap ...
//!   EXIT` to remove it.
//!
//! The remote host is always assumed Linux/macOS, so `remote_sock` is
//! always a Unix socket path and the wrapped remote command is always a
//! POSIX shell script, regardless of the *local* platform. Locally,
//! `bind_local_listener` uses a Unix socket on macOS/Linux, or a loopback
//! TCP socket on Windows (`ssh -R` supports mixing a Unix-socket endpoint
//! with a TCP endpoint on the other side; verified against a real Windows
//! OpenSSH client via `scripts/spike-ssh-forward-windows.ps1` — see
//! `agent::LocalListener`).

mod agent;
mod dir_transfer;
#[cfg(test)]
mod integration_tests;
mod protocol;
// `remote_client` dials the forwarded socket via a Unix stream: it only
// ever runs on the *remote* end of a session, which is always assumed
// Linux/macOS (see module docs), so it is unconditionally unix-only —
// unlike `agent`, which must compile on Windows too since Windows is
// supported as the *local* side.
#[cfg(unix)]
mod remote_client;

use anyhow::{Context, Result, bail};

use crate::config::Config;

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
) -> Result<()> {
    remote_client::handle_download(remote_source, local_destination, force, dry_run).await
}

#[cfg(not(unix))]
pub async fn handle_local_download(
    _remote_source: &str,
    _local_destination: Option<&str>,
    _force: bool,
    _dry_run: bool,
) -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(unix)]
pub async fn handle_local_upload(
    local_source: &str,
    remote_destination: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    remote_client::handle_upload(local_source, remote_destination, force, dry_run).await
}

#[cfg(not(unix))]
pub async fn handle_local_upload(
    _local_source: &str,
    _remote_destination: Option<&str>,
    _force: bool,
    _dry_run: bool,
) -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

#[cfg(unix)]
pub async fn handle_local_status() -> Result<()> {
    remote_client::handle_status().await
}

#[cfg(not(unix))]
pub async fn handle_local_status() -> Result<()> {
    bail!(WINDOWS_REMOTE_UNSUPPORTED)
}

/// Single-letter ssh options that consume a separate value, per ssh(1).
/// Used only to locate the destination/command boundary in the user's
/// argument list — never to reinterpret what the options mean.
const VALUE_OPTION_LETTERS: &[char] = &[
    'B', 'b', 'c', 'D', 'E', 'e', 'F', 'I', 'i', 'J', 'L', 'l', 'm', 'O', 'o', 'p', 'Q', 'R', 'S',
    'W', 'w',
];

pub async fn handle_ssh(config: &Config, args: &[String]) -> Result<()> {
    let (ssh_options, host, remote_command) = split_ssh_args(args)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();

    let session_dir = config.shine_dir().join("run").join("ssh").join(&session_id);
    tokio::fs::create_dir_all(&session_dir)
        .await
        .with_context(|| format!("creating {}", session_dir.display()))?;
    // The remote host is always assumed Linux/macOS (see module docs), so
    // its socket is always a Unix socket regardless of the local platform.
    let remote_sock = format!("/tmp/.shine-ssh-{session_id}.sock");

    let (listener, local_forward_target) = bind_local_listener(&session_dir).await?;
    let session_local_dir = std::env::current_dir().context("reading current directory")?;

    let agent_handle = tokio::spawn(listener.serve(token.clone(), session_local_dir.clone()));

    let wrapped_command =
        build_wrapped_remote_command(&session_id, &token, &remote_sock, &remote_command);

    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(build_ssh_invocation_args(
        &ssh_options,
        &remote_sock,
        &local_forward_target,
        &host,
        &wrapped_command,
    ));

    // Racing against ctrl_c() (rather than just awaiting cmd.status()) is
    // what makes the cleanup below actually run on Ctrl-C: installing this
    // listener overrides SIGINT's default disposition for the process, so a
    // Ctrl-C no longer kills us before we get a chance to clean up. The ssh
    // child is in the same foreground process group and receives SIGINT
    // independently; we still await its exit so we don't race it.
    let mut ssh_run = std::pin::pin!(cmd.status());
    let status = tokio::select! {
        status = &mut ssh_run => status,
        _ = tokio::signal::ctrl_c() => ssh_run.await,
    }
    .context("failed to run ssh")?;

    agent_handle.abort();
    let _ = tokio::fs::remove_dir_all(&session_dir).await;

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

/// Splits a raw `shine ssh` argument list into ssh options, the destination,
/// and an optional remote command — mirroring what `ssh` itself would infer,
/// without reinterpreting the options' meaning (see module docs). An
/// explicit `--` may be used to disambiguate; it is consumed here and not
/// forwarded to the real `ssh` invocation.
fn split_ssh_args(args: &[String]) -> Result<(Vec<String>, String, Vec<String>)> {
    let mut ssh_options = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let token = &args[i];
        if token == "--" {
            i += 1;
            break;
        }
        if token == "-" || !token.starts_with('-') {
            let host = token.clone();
            let remote_command = args[i + 1..].to_vec();
            return Ok((ssh_options, host, remote_command));
        }

        ssh_options.push(token.clone());
        let letters: Vec<char> = token.chars().skip(1).collect();
        let mut consumes_next = false;
        for (idx, letter) in letters.iter().enumerate() {
            if VALUE_OPTION_LETTERS.contains(letter) {
                consumes_next = idx == letters.len() - 1;
                break;
            }
        }
        i += 1;
        if consumes_next {
            let Some(value) = args.get(i) else {
                bail!("ssh option {token} requires a value");
            };
            ssh_options.push(value.clone());
            i += 1;
        }
    }

    let Some(host) = args.get(i) else {
        bail!("no SSH destination given; usage: shine ssh [SSH_ARGS]... <HOST> [COMMAND]");
    };
    let remote_command = args[i + 1..].to_vec();
    Ok((ssh_options, host.clone(), remote_command))
}

/// Assembles the argument list passed to the `ssh` binary: the user's own
/// options first (untouched, per module docs), then our `-t`/`-R` forward,
/// the destination, and the wrapped remote command. Kept as a pure function
/// so the composition can be unit-tested without spawning a real `ssh`.
fn build_ssh_invocation_args(
    ssh_options: &[String],
    remote_sock: &str,
    local_forward_target: &str,
    host: &str,
    wrapped_command: &str,
) -> Vec<String> {
    let mut args = ssh_options.to_vec();
    args.push("-t".to_string());
    args.push("-R".to_string());
    args.push(format!("{remote_sock}:{local_forward_target}"));
    args.push(host.to_string());
    args.push(wrapped_command.to_string());
    args
}

fn build_wrapped_remote_command(
    session_id: &str,
    token: &str,
    remote_sock: &str,
    remote_command: &[String],
) -> String {
    let inner_exec = if remote_command.is_empty() {
        r#"exec "$SHELL" -l"#.to_string()
    } else {
        let quoted = remote_command
            .iter()
            .map(|token| single_quote(token))
            .collect::<Vec<_>>()
            .join(" ");
        format!("exec {quoted}")
    };
    // Double quotes here are safe: this text is only ever embedded through
    // `single_quote`, which POSIX-escapes it as one opaque literal for the
    // outer shell, so nothing inside (single or double quotes, `$`, etc.)
    // is interpreted until the inner `sh -c` re-parses it.
    let inner_script = format!(r#"trap "rm -f $SHINE_SSH_REMOTE_SOCK" EXIT; {inner_exec}"#);

    format!(
        "env SHINE_SSH_SESSION={session_id} SHINE_SSH_TOKEN={token} SHINE_SSH_REMOTE_SOCK={remote_sock} sh -c {}",
        single_quote(&inner_script)
    )
}

/// POSIX single-quotes `s` for safe embedding as one shell word, escaping
/// any literal `'` via the standard `'\''` idiom (close quote, escaped
/// quote, reopen quote). Applying this at each nesting level independently
/// composes correctly regardless of how many quoting layers are involved.
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_host_with_no_options_or_command() {
        let (options, host, command) = split_ssh_args(&["dev".to_string()]).unwrap();
        assert!(options.is_empty());
        assert_eq!(host, "dev");
        assert!(command.is_empty());
    }

    #[test]
    fn host_followed_by_a_remote_command() {
        let args = vec!["dev".to_string(), "ls".to_string(), "-la".to_string()];
        let (options, host, command) = split_ssh_args(&args).unwrap();
        assert!(options.is_empty());
        assert_eq!(host, "dev");
        assert_eq!(command, vec!["ls", "-la"]);
    }

    #[test]
    fn value_option_with_separate_token() {
        let args = vec!["-p".to_string(), "2222".to_string(), "dev".to_string()];
        let (options, host, command) = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-p", "2222"]);
        assert_eq!(host, "dev");
        assert!(command.is_empty());
    }

    #[test]
    fn value_option_with_attached_value() {
        let args = vec!["-p2222".to_string(), "dev".to_string()];
        let (options, host, _command) = split_ssh_args(&args).unwrap();
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
        let (options, host, _command) = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-o", "ProxyJump=bastion"]);
        assert_eq!(host, "dev");
    }

    #[test]
    fn bundled_boolean_flags_consume_no_value() {
        let args = vec!["-vvv".to_string(), "dev".to_string()];
        let (options, host, _command) = split_ssh_args(&args).unwrap();
        assert_eq!(options, vec!["-vvv"]);
        assert_eq!(host, "dev");
    }

    #[test]
    fn explicit_double_dash_separator() {
        let args = vec!["--".to_string(), "dev".to_string(), "ls".to_string()];
        let (options, host, command) = split_ssh_args(&args).unwrap();
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

    #[test]
    fn wrapped_command_round_trips_through_a_real_shell() {
        // Exercises the full nested-quoting composition end to end: run the
        // wrapped command through `sh -c` and check the remote command's
        // stdout, rather than reasoning about escaping by hand.
        let wrapped = build_wrapped_remote_command(
            "sid",
            "tok",
            "/tmp/shine-ssh-mod-test-sid.sock",
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
        let (parsed_options, host, _command) =
            split_ssh_args(&[args.clone(), vec!["dev".to_string()]].concat()).unwrap();

        let invocation = build_ssh_invocation_args(
            &parsed_options,
            "/tmp/.shine-ssh-sid.sock",
            "/tmp/shine-ssh-sid/local.sock",
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
        let (parsed_options, host, command) = split_ssh_args(&args).unwrap();
        assert_eq!(command, vec!["ls", "-la"]);

        let invocation = build_ssh_invocation_args(
            &parsed_options,
            "/tmp/.shine-ssh-sid.sock",
            "/tmp/shine-ssh-sid/local.sock",
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
        let wrapped = build_wrapped_remote_command("sid", "tok", "/tmp/.shine-ssh-sid.sock", &[]);
        assert!(wrapped.contains(r#"exec "$SHELL" -l"#));
        assert!(wrapped.contains("trap \"rm -f $SHINE_SSH_REMOTE_SOCK\" EXIT"));
    }
}
