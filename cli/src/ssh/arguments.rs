//! Exact SSH option/destination boundary and transport argv; never reinterpret options.
use super::*;

/// Single-letter ssh options that consume a separate value, per ssh(1).
/// Used only to locate the destination/command boundary in the user's
/// argument list — never to reinterpret what the options mean.
pub(super) const VALUE_OPTION_LETTERS: &[char] = &[
    'B', 'b', 'c', 'D', 'E', 'e', 'F', 'I', 'i', 'J', 'L', 'l', 'm', 'O', 'o', 'p', 'Q', 'R', 'S',
    'W', 'w',
];

pub(super) struct ParsedSshArgs {
    pub options: Vec<String>,
    pub host: String,
    pub remote_command: Vec<String>,
}

/// Splits a raw `shine ssh` argument list into ssh options, the destination,
/// and an optional remote command — mirroring what `ssh` itself would infer,
/// without reinterpreting the options' meaning (see module docs). An
/// explicit `--` may be used to disambiguate; it is consumed here and not
/// forwarded to the real `ssh` invocation.
pub(super) fn split_ssh_args(args: &[String]) -> Result<ParsedSshArgs> {
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
            return Ok(ParsedSshArgs {
                options: ssh_options,
                host,
                remote_command,
            });
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
    Ok(ParsedSshArgs {
        options: ssh_options,
        host: host.clone(),
        remote_command,
    })
}

/// Assembles the argument list passed to the `ssh` binary: the user's own
/// options first (untouched, per module docs), then our `-t`/`-R` forward,
/// the destination, and the wrapped remote command. Kept as a pure function
/// so the composition can be unit-tested without spawning a real `ssh`.
pub(super) fn build_ssh_invocation_args(
    ssh_options: &[String],
    remote_sock: &str,
    local_forward_target: &str,
    control_path: Option<&std::path::Path>,
    host: &str,
    wrapped_command: &str,
) -> Vec<String> {
    let mut args = ssh_options.to_vec();
    // Enable connection multiplexing so a later `rsync`/`scp` child can reuse
    // this authenticated master connection (ADR 0011). Only injected when the
    // user didn't set their own ControlMaster/ControlPath.
    if let Some(control_path) = control_path {
        args.push("-o".to_string());
        args.push("ControlMaster=auto".to_string());
        args.push("-o".to_string());
        args.push(format!("ControlPath={}", control_path.display()));
        args.push("-o".to_string());
        args.push("ControlPersist=60".to_string());
    }
    args.push("-t".to_string());
    args.push("-R".to_string());
    args.push(format!("{remote_sock}:{local_forward_target}"));
    args.push(host.to_string());
    args.push(wrapped_command.to_string());
    args
}

/// Windows does not receive a transfer listener, so its SSH invocation is a
/// deliberately small normal TTY session followed by one opaque PowerShell
/// command. Keeping the encoded payload as a single argv item prevents CMD
/// from seeing secret values or PowerShell metacharacters.
pub(super) fn build_windows_ssh_invocation_args(
    ssh_options: &[String],
    host: &str,
    wrapped_command: &str,
) -> Vec<String> {
    let mut args = ssh_options.to_vec();
    args.push("-t".to_string());
    args.push(host.to_string());
    args.push(wrapped_command.to_string());
    args
}
