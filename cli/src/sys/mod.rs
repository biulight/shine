mod commands;
mod detect;
mod execution;
mod managed;
#[cfg(test)]
mod manifest;
mod model;
mod profile_commands;
mod recovery;
mod render;
mod run_manifest;
mod selection;

#[cfg(test)]
use model::SysInstalledRow;
use model::{
    LoadedSysPreset, ResolvedSelection, SysDetection, SysDetectionProbe, SysDriverKind, SysInstall,
    SysItem, SysItemMode, SysItemOutcome, SysItemStatus, SysPackageProvider, SysUpdateRow,
    SysUpgradeReport,
};

use anyhow::{Context, Result};

pub use commands::{BootstrapCliOptions, handle_info, handle_init, handle_list, handle_status};
pub use detect::detect_os_id;
pub use managed::{
    handle_apply, handle_apply_approved, handle_uninstall, handle_uninstall_approved,
    handle_upgrade_managed, handle_upgrade_managed_target, managed_updates,
};
pub(crate) use managed::{
    handle_upgrade_managed_target_with_result_approved, handle_upgrade_managed_with_result_prepared,
};
pub use profile_commands::{
    handle_profile_disable, handle_profile_disable_approved, handle_profile_enable,
    handle_profile_enable_approved,
};
pub use recovery::handle_recover_approved;

const SYS_TEMPLATE: &str = r#"# System bootstrap preset metadata for shine (schema v2).
version = 2
description = "My system bootstrap."
default_profile = "recommended"

[[items]]
id = "my-tool"
label = "My Tool"
description = "Install and configure my tool."
default = true
detect = { kind = "command", command = "my-tool", version_args = ["--version"] }
# Keep scripts inside this category. For Windows, use a .ps1 script instead.
install = { kind = "script", path = "install/my-tool.sh" }

# Optional author capability notes do not restrict the script. Declare environment inputs and
# administrator requirements when Shine must inject or elevate them.
# [items.permissions]
# schema_version = 1
# environment = [{ name = "API_TOKEN", sensitivity = "secret" }]

[profiles.recommended]
items = ["my-tool"]
"#;

pub async fn handle_init_template(force: bool, unrestricted: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let template = init_template(unrestricted);
    let (path, overwritten) =
        shine_core::init_template::write_shine_toml_template(&dir, force, &template)?;
    if overwritten {
        println!("Updated sys preset template: {}", path.display());
    } else {
        println!("Created sys preset template: {}", path.display());
    }
    Ok(())
}

fn init_template(unrestricted: bool) -> String {
    if unrestricted {
        format!(
            "{SYS_TEMPLATE}\n[permission_defaults]\nschema_version = 2\nopaque_code = \"unrestricted\"\n"
        )
    } else {
        SYS_TEMPLATE.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_template_does_not_require_a_permission_declaration() {
        let manifest = manifest::parse_and_validate_manifest(SYS_TEMPLATE).unwrap();

        assert_eq!(manifest.items.len(), 1);
        assert!(manifest.items[0].permissions.is_none());
    }

    #[test]
    fn unrestricted_init_template_contains_valid_permission_defaults() {
        let manifest = manifest::parse_and_validate_manifest(&init_template(true)).unwrap();
        let permissions = manifest.items[0].permissions.as_ref().unwrap();
        assert_eq!(permissions.schema_version, 2);
        assert!(permissions.opaque_code.is_some());
    }
}
