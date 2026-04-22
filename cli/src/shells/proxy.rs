use crate::config::Config;
use crate::shells::get_shell_config_path;
use anyhow::{Context, Result};
use tokio::fs;

pub(super) async fn install_proxy(config: &Config) -> Result<()> {
    let shell_config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    println!("Initializing shell config: {:?}", shell_config_path);

    fs::create_dir_all(config.presets_dir())
        .await
        .context("creating presets directory")?;

    let report = crate::presets::extract_prefix("shell/proxy", config.presets_dir(), false).await?;

    println!(
        "Presets: {} created, {} skipped",
        report.created.len(),
        report.skipped.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::fs;

    #[tokio::test]
    async fn install_proxy_creates_preset_files() {
        let dir = std::env::temp_dir().join(format!("shine-proxy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();

        let config = Config::new_for_test(&dir);
        install_proxy(&config).await.unwrap();

        let set_proxy = config
            .presets_dir()
            .join("shell")
            .join("proxy")
            .join("set_proxy.sh");
        let unset_proxy = config
            .presets_dir()
            .join("shell")
            .join("proxy")
            .join("uset_proxy.sh");

        assert!(set_proxy.exists(), "set_proxy.sh should exist");
        assert!(unset_proxy.exists(), "uset_proxy.sh should exist");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
