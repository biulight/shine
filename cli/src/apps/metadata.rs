//! Thin CLI adapter for Core-owned App preset metadata.

use crate::config::Config;
use anyhow::Result;

pub use utils::runtime::{
    AppCategory, AppDestinationRoot, AppFile, AppGenerator, AppHook, AppListMode,
};

pub fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<AppCategory>> {
    crate::core_runtime::from_embedded_presets().app_categories(filter)
}

pub async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<AppCategory>> {
    crate::core_runtime::from_installed_presets(config)
        .await?
        .app_categories(filter)
}

pub async fn load_active_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<AppCategory>> {
    crate::core_runtime::from_config(config)
        .await?
        .app_categories(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_metadata_is_parsed_by_core() {
        let categories = load_embedded_categories(Some("vim")).unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "vim");
        assert!(!categories[0].files.is_empty());
    }
}
