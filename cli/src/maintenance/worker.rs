use super::Scope;
use anyhow::Result;
use std::process::{Command, Stdio};
use uuid::Uuid;

pub(super) fn spawn(scope: &Scope, ticket: Uuid) -> Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--config-dir")
        .arg(&scope.config_dir)
        .args([
            "__maintenance-worker",
            "--scope",
            &scope.key,
            "--ticket",
            &ticket.to_string(),
        ])
        .current_dir(&scope.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; the closure does no allocation or locking.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    let mut child = tokio::process::Command::from(command).spawn()?;
    // Reap while the caller is alive. Dropping this task at CLI exit does not kill the worker.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}
