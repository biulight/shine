//! CLI-only administrator helpers for self-installation.

use anyhow::Result;
use std::io::IsTerminal;

pub(crate) async fn admin_lock() -> Result<utils::runtime::PrivilegedOperationGuard> {
    use utils::runtime::PrivilegedFileSystemHost;
    utils::runtime::RealHost
        .acquire_privileged_operation()
        .await
}

/// Builds a sudo command which fails fast rather than prompting without a TTY.
pub(crate) fn sudo_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sudo");
    if !std::io::stdin().is_terminal() {
        command.arg("-n");
    }
    command
}
