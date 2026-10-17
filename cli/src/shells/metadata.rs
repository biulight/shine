//! Thin CLI adapter for Core-owned Shell preset metadata and target parsing.

use crate::config::Config;
use anyhow::Result;

pub use shine_core::runtime::parse_shell_lifecycle_target as parse_lifecycle_target;
pub use shine_core::runtime::{ShellCategory, ShellFile};

pub fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<ShellCategory>> {
    crate::core_runtime::from_embedded_presets().shell_categories(filter)
}

pub async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<ShellCategory>> {
    crate::core_runtime::from_installed_presets(config)
        .await?
        .shell_categories(filter)
}

pub async fn load_active_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<ShellCategory>> {
    crate::core_runtime::from_config(config)
        .await?
        .shell_categories(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_metadata_is_parsed_by_core() {
        let categories = load_embedded_categories(Some("proxy")).unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "proxy");
        assert!(!categories[0].files.is_empty());
    }
}
