use anyhow::{Context, Result};
use dialoguer::Confirm;
use std::io::IsTerminal;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

static AUTHORIZED: AtomicBool = AtomicBool::new(false);

pub async fn ensure_admin(action_count: usize) -> Result<bool> {
    if AUTHORIZED.load(Ordering::Acquire) {
        return Ok(true);
    }
    if cfg!(windows) {
        // Windows drivers batch their privileged work through an elevated
        // PowerShell process. Authorization happens when that batch starts.
        return Ok(true);
    }
    if std::env::var("USER").is_ok_and(|user| user == "root") {
        AUTHORIZED.store(true, Ordering::Release);
        return Ok(true);
    }
    if tokio::process::Command::new("sudo")
        .args(["-n", "-v"])
        .status()
        .await
        .is_ok_and(|status| status.success())
    {
        AUTHORIZED.store(true, Ordering::Release);
        return Ok(true);
    }
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        eprintln!(
            "managed system configuration requires administrator privileges; run interactively"
        );
        return Ok(false);
    }
    let confirmed = Confirm::new()
        .with_prompt(format!(
            "Apply {action_count} operation(s) with administrator privileges?"
        ))
        .default(false)
        .interact()?;
    if !confirmed {
        return Ok(false);
    }
    let status = tokio::process::Command::new("sudo")
        .arg("-v")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to request administrator privileges")?;
    if status.success() {
        AUTHORIZED.store(true, Ordering::Release);
    }
    Ok(status.success())
}
