//! Pure POSIX and encoded PowerShell remote wrappers. No session resources or execution.
use super::*;

pub(super) fn build_wrapped_remote_command(
    session_id: &str,
    token: &str,
    remote_sock: &str,
    local_theme: Option<&str>,
    forwarded_env: &BTreeMap<String, String>,
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

    let mut env_prefix = format!(
        "SHINE_SSH_SESSION={session_id} SHINE_SSH_TOKEN={token} SHINE_SSH_REMOTE_SOCK={remote_sock}"
    );
    // Unlike the three values above (internally generated UUIDs/hex/paths,
    // never user input), this one is quoted defensively per
    // docs/terminal-theme-sync-prd.md §6.1/§10 even though its source
    // (`Theme::as_str`) only ever produces the literal `light` or `dark`.
    if let Some(theme) = local_theme {
        env_prefix.push_str(&format!(" SHINE_TERMINAL_THEME={}", single_quote(theme)));
    }
    for (key, value) in forwarded_env {
        env_prefix.push_str(&format!(" {key}={}", single_quote(value)));
    }

    format!("env {env_prefix} sh -c {}", single_quote(&inner_script))
}

/// Builds an opaque Windows PowerShell remote command. OpenSSH passes this
/// through cmd.exe on typical Windows servers, so only the Base64 alphabet is
/// allowed to carry user-controlled values across that boundary. A small
/// Windows PowerShell bootstrap probes for PowerShell 7 before launching the
/// real encoded payload in the selected shell. Keeping the outer command free
/// of CMD operators also works when the SSH server's default shell has already
/// been changed from CMD to PowerShell.
pub(super) fn build_windows_wrapped_remote_command(
    session_id: &str,
    local_theme: Option<&str>,
    forwarded_env: &BTreeMap<String, String>,
    remote_command: &[String],
) -> Result<String> {
    let mut script = String::new();
    push_powershell_env_assignment(&mut script, "SHINE_SSH_SESSION", session_id)?;
    if let Some(theme) = local_theme {
        push_powershell_env_assignment(&mut script, "SHINE_TERMINAL_THEME", theme)?;
    }
    for (key, value) in forwarded_env {
        push_powershell_env_assignment(&mut script, key, value)?;
    }

    let interactive = remote_command.is_empty();
    if !interactive {
        // Reset the native-program status so a PowerShell command does not
        // accidentally inherit a status from profile/startup execution.
        script.push_str("$global:LASTEXITCODE = 0\n& ");
        for (index, argument) in remote_command.iter().enumerate() {
            if index > 0 {
                script.push(' ');
            }
            script.push_str(&powershell_single_quoted_literal(argument)?);
        }
        script.push_str(
            "\nif ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }\nif (-not $?) { exit 1 }\n",
        );
    }

    let payload_encoded = BASE64.encode(utf16le_bytes(&script)?);
    let no_profile = if interactive { "" } else { " -NoProfile" };
    let no_exit = if interactive { " -NoExit" } else { "" };
    let bootstrap = format!(
        "$pwsh = Get-Command pwsh.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1\n\
         $shell = if ($null -ne $pwsh) {{ $pwsh.Source }} else {{ 'powershell.exe' }}\n\
         & $shell{no_profile}{no_exit} -EncodedCommand '{payload_encoded}'\n\
         $ok = $?\n\
         $code = $LASTEXITCODE\n\
         if ($null -ne $code -and $code -ne 0) {{ exit $code }}\n\
         if (-not $ok) {{ exit 1 }}\n"
    );
    let bootstrap_encoded = BASE64.encode(utf16le_bytes(&bootstrap)?);
    Ok(format!(
        "powershell.exe -NoProfile -EncodedCommand {bootstrap_encoded}"
    ))
}

pub(super) fn push_powershell_env_assignment(
    script: &mut String,
    key: &str,
    value: &str,
) -> Result<()> {
    // Targets have already been parsed as environment identifiers by
    // `parse_env_specs`; keep this check local so this builder remains safe
    // if it is reused independently later.
    if !key
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("cannot safely represent Windows environment variable name {key}");
    }
    script.push_str("$env:");
    script.push_str(key);
    script.push_str(" = ");
    script.push_str(&powershell_single_quoted_literal(value)?);
    script.push('\n');
    Ok(())
}

/// Produces a PowerShell single-quoted string. PowerShell represents a literal
/// apostrophe by doubling it; NUL is rejected because Windows environment
/// values and command-line APIs cannot represent it safely.
pub(super) fn powershell_single_quoted_literal(value: &str) -> Result<String> {
    if value.contains('\0') {
        bail!("cannot forward values containing NUL bytes to Windows PowerShell");
    }
    Ok(shine_core::shell_quote::powershell_single_quote(value))
}

pub(super) fn utf16le_bytes(value: &str) -> Result<Vec<u8>> {
    if value.contains('\0') {
        bail!("cannot encode PowerShell commands containing NUL bytes");
    }
    Ok(value
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>())
}

/// POSIX single-quotes `s` for safe embedding as one shell word, escaping
/// any literal `'` via the standard `'\''` idiom (close quote, escaped
/// quote, reopen quote). Applying this at each nesting level independently
/// composes correctly regardless of how many quoting layers are involved.
pub(super) use crate::shell_quote::single_quote;
