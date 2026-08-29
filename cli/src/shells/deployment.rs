use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::Result;

#[cfg(test)]
pub(crate) use utils::runtime::{ShellManifest, ShellManifestEntry};

pub async fn handle_render_live(config: &Config, target: &str) -> Result<()> {
    let mut runtime = crate::core_runtime::from_config(config).await?;
    runtime.context_mut_for_cli().env = EnvConfig::load_or_init(config).await?.as_map().clone();
    runtime.render_live_shell(target).await
}
