mod commands;
mod detect;
mod drivers;
mod execution;
mod managed;
mod manifest;
mod model;
mod profile;
mod profile_blocks;
mod render;
mod resources;
mod run_manifest;
mod selection;

use commands::{SYS_STATUS_PREFIX, SYS_UPDATE_PREFIX};
use managed::SysAction;
use model::{
    LoadedSysPreset, ResolvedSelection, SYS_PROFILE_PHASES, SelectionSource,
    ShellProfileBlockPosition, SysDriverKind, SysInitCommand, SysInstalledRow, SysItem,
    SysItemMode, SysItemOutcome, SysItemStatus, SysManifest, SysProfilePhase, SysUpdateCheck,
    SysUpdateRow, SysUpdateState, SysUpgradeReport,
};
use render::sys_init_theme;

pub use commands::{handle_info, handle_init, handle_list, handle_status, handle_update};
pub use detect::detect_os_id;
pub(crate) use managed::installed_managed;
pub use managed::{handle_apply, handle_uninstall, handle_upgrade_managed, managed_updates};
