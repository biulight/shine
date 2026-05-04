use crate::config::Config;
use anyhow::Result;

#[derive(Debug, Default)]
pub(crate) struct EnvUpgradeReport {
    pub updated: usize,
    pub skipped: usize,
    pub user_modified: usize,
}

pub(crate) async fn handle_upgrade(
    _config: &Config,
    _dry_run: bool,
    _verbose: bool,
) -> Result<EnvUpgradeReport> {
    // Env-backed templates are upgraded by their owning subsystems:
    // shell presets under `Shell Presets`, app configs under `App Configs`.
    Ok(EnvUpgradeReport::default())
}
