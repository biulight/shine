//! Pure argv construction; wire values are operands, trusted context owns SSH options.

use crate::ssh::{
    protocol::{Direction, Tool},
    session_context::SessionContext,
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// A validated, resolved transfer ready to be turned into an argv.
pub(super) struct TransferPlan {
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
pub(super) struct TransferCommand {
    pub program: String,
    pub args: Vec<String>,
    pub(super) tool: Tool,
}

impl TransferCommand {
    pub(super) fn process(&self) -> tokio::process::Command {
        let mut process = tokio::process::Command::new(&self.program);
        process.args(&self.args);
        if self.tool == Tool::Rsync {
            // We escape the remote shell operand ourselves for compatibility with
            // macOS rsync. Newer rsync must not escape it a second time. Older
            // versions ignore OLD_ARGS; PROTECT_ARGS also pins their behavior.
            process
                .env("RSYNC_OLD_ARGS", "1")
                .env("RSYNC_PROTECT_ARGS", "0");
        }
        process
    }
}

/// Builds the rsync/scp argv (the pure, unit-tested seam). Never lets wire input
/// reach an option position: the remote operand is always `<host>:<remote_spec>`
/// after a `--`, and the ssh reconnection string is built solely from `ctx`.
pub(super) fn build_transfer_argv(
    tool: Tool,
    ctx: &SessionContext,
    plan: &TransferPlan,
) -> Result<TransferCommand> {
    if plan.remote_spec.contains(['\0', '\n', '\r']) {
        bail!("remote path contains an illegal control character");
    }
    let remote_spec = if tool == Tool::Rsync {
        escape_rsync_remote_spec(&plan.remote_spec)
    } else {
        plan.remote_spec.clone()
    };
    let remote_operand = format!("{}:{remote_spec}", transfer_host(&ctx.host));
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
            // macOS ships an rsync 2.6.9-compatible implementation without --info.
            args.push("--progress".to_string());
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
            for opt in scp_connection_options(ctx)? {
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
        tool,
    })
}

pub(super) fn ipv6_destination(host: &str) -> Option<(Option<&str>, &str)> {
    let (user, address) = match host.rsplit_once('@') {
        Some((user, address)) => (Some(user), address),
        None => (None, host),
    };
    let address = address
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(address);
    let literal = address
        .split_once('%')
        .map_or(address, |(literal, _)| literal);
    literal.parse::<std::net::Ipv6Addr>().ok()?;
    Some((user, address))
}

pub(super) fn transfer_host(host: &str) -> String {
    match ipv6_destination(host) {
        Some((Some(user), address)) => format!("{user}@[{address}]"),
        Some((None, address)) => format!("[{address}]"),
        None => host.to_string(),
    }
}

/// Preserve remote glob expansion while keeping whitespace and shell syntax in
/// the path literal. A local argv separator does not protect the remote shell.
fn escape_rsync_remote_spec(spec: &str) -> String {
    let mut escaped = String::with_capacity(spec.len());
    for ch in spec.chars() {
        if !(ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '/' | '.'
                    | '_'
                    | '-'
                    | ':'
                    | '@'
                    | '+'
                    | ','
                    | '%'
                    | '='
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '!'
                    | '^'
            ))
        {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
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
pub(super) fn rsync_rsh(ctx: &SessionContext) -> Result<Option<String>> {
    let options = ssh_reconnect_options(ctx);
    if options.is_empty() {
        return Ok(None);
    }
    if options.iter().any(|o| o.chars().any(char::is_whitespace)) {
        bail!("ssh options contain whitespace and can't be passed to rsync -e");
    }
    Ok(Some(format!("ssh {}", options.join(" "))))
}

/// Translate trusted SSH connection options into SCP options. Several short
/// options have different meanings in SCP; session/forwarding flags are omitted.
pub(super) fn scp_connection_options(ctx: &SessionContext) -> Result<Vec<String>> {
    let options = ssh_reconnect_options(ctx);
    let mut tokens = options.iter();
    let mut args = Vec::new();
    while let Some(token) = tokens.next() {
        let letters = token
            .strip_prefix('-')
            .filter(|letters| !letters.is_empty() && !letters.starts_with('-'))
            .context("invalid SSH reconnection option")?;
        let mut letters = letters.char_indices().peekable();
        while let Some((_, letter)) = letters.next() {
            if crate::ssh::VALUE_OPTION_LETTERS.contains(&letter) {
                let value = if let Some((offset, _)) = letters.peek() {
                    &token[1 + offset..]
                } else {
                    tokens
                        .next()
                        .with_context(|| format!("SSH option -{letter} requires a value"))?
                        .as_str()
                };
                let setting = match letter {
                    'B' => Some("BindInterface"),
                    'b' => Some("BindAddress"),
                    'I' => Some("PKCS11Provider"),
                    'l' => Some("User"),
                    'm' => Some("MACs"),
                    'p' => Some("Port"),
                    'S' => Some("ControlPath"),
                    'c' | 'F' | 'i' | 'J' | 'o' => {
                        args.extend([format!("-{letter}"), value.to_string()]);
                        None
                    }
                    // These affect the original session, not its transfer connection.
                    'D' | 'E' | 'e' | 'L' | 'O' | 'Q' | 'R' | 'W' | 'w' => None,
                    _ => bail!("unsupported SSH reconnection option -{letter}"),
                };
                if let Some(setting) = setting {
                    args.extend(["-o".to_string(), format!("{setting}={value}")]);
                }
                break;
            }
            match letter {
                '4' | '6' | 'A' | 'C' | 'q' | 'v' => args.push(format!("-{letter}")),
                'a' => args.extend(["-o".into(), "ForwardAgent=no".into()]),
                'K' => args.extend([
                    "-o".into(),
                    "GSSAPIAuthentication=yes".into(),
                    "-o".into(),
                    "GSSAPIDelegateCredentials=yes".into(),
                ]),
                'k' => args.extend(["-o".into(), "GSSAPIDelegateCredentials=no".into()]),
                'f' | 'G' | 'g' | 'M' | 'N' | 'n' | 's' | 'T' | 't' | 'V' | 'X' | 'x' | 'Y'
                | 'y' => {}
                _ => bail!("unsupported SSH reconnection option -{letter}"),
            }
        }
    }
    Ok(args)
}

/// Resolves the binary for a tool, honoring a per-tool test-override env var so
/// integration tests can point at a stub script.
pub(super) fn resolve_tool_binary(tool: Tool) -> String {
    let (var, default) = match tool {
        Tool::Rsync => ("SHINE_TEST_RSYNC", "rsync"),
        Tool::Scp => ("SHINE_TEST_SCP", "scp"),
    };
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Chooses rsync (default) or scp, reporting whether we fell back and a note to
/// show the user.
pub(super) fn choose_tool(
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
