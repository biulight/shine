use crate::config::Config;
use anyhow::Result;

#[cfg(test)]
pub(crate) use shine_core::runtime::{ShellManifest, ShellManifestEntry};

pub async fn handle_render_live(config: &Config, target: &str) -> Result<()> {
    let runtime = crate::core_runtime::from_config(config).await?;
    runtime.render_live_shell(target).await
}

/// Launch from one installed receipt and the caller's already layered config.
pub async fn handle_launch_live(
    config: &Config,
    target: &str,
    args: &[std::ffi::OsString],
) -> Result<()> {
    let launch = {
        let runtime = crate::core_runtime::from_config(config).await?;
        runtime.prepare_live_bun_launch(target).await?
    }; // Drop the captured preset tree before decryption and long-lived Bun.
    crate::env::live_bun::run_declared_bun(config, launch, args).await
}
