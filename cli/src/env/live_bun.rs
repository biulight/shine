//! Receipt-prepared Bun launch; no workspace discovery or nested Shine invocation.
use super::workspace::{command_builder, finish_command_status, resolve_explicit_values};
use crate::config::Config;
use anyhow::{Context, Result};
use std::{collections::BTreeMap, ffi::OsString};
use zeroize::Zeroize;

pub(crate) async fn run_declared_bun(
    config: &Config,
    launch: shine_core::runtime::LiveBunLaunch,
    args: &[OsString],
) -> Result<()> {
    let mut explicit = if launch.env.is_empty() {
        BTreeMap::new()
    } else {
        resolve_explicit_values(config, &launch.env).await?
    };
    let mut command = vec![
        OsString::from("bun"),
        OsString::from(launch.dependencies.install_arg()),
        launch.script.into_os_string(),
    ];
    command.extend_from_slice(args);
    let result = launch_status(&command, &explicit).await;
    for value in explicit.values_mut() {
        value.zeroize();
    }
    match result {
        Ok(status) => finish_command_status(status),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            eprintln!("shine: Bun was not found on PATH. Install Bun from https://bun.sh");
            std::process::exit(127);
        }
        Err(error) => Err(error),
    }
}

async fn launch_status(
    command: &[OsString],
    explicit: &BTreeMap<String, String>,
) -> Result<std::process::ExitStatus> {
    #[cfg(unix)]
    let mut signals = [
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?,
    ];
    let mut child = command_builder(command, &BTreeMap::new(), false, explicit)?
        .kill_on_drop(true)
        .spawn()
        .context("starting Bun")?;
    #[cfg(unix)]
    loop {
        let [interrupt, terminate, hangup] = &mut signals;
        let signal = tokio::select! {
            status = child.wait() => return status.context("waiting for Bun"),
            _ = interrupt.recv() => libc::SIGINT,
            _ = terminate.recv() => libc::SIGTERM,
            _ = hangup.recv() => libc::SIGHUP,
        };
        if let Some(pid) = child.id() {
            // The child keeps the caller's foreground process group/terminal.
            // Also relay signals sent to the waiting Shine PID alone. No new
            // process group or descendant-containment contract is introduced.
            unsafe {
                libc::kill(pid as libc::pid_t, signal);
            }
        }
    }
    #[cfg(not(unix))]
    child.wait().await.context("waiting for Bun")
}
