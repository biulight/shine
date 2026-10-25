//! Stable command facade for version checks, binary installation and configuration upgrades.

mod binary;
mod config_upgrade;
mod release;
mod update;

pub use binary::handle_self_install;
pub(crate) use config_upgrade::finish_converged_upgrade;
pub use config_upgrade::handle_config_upgrade;
pub use release::handle_self_upgrade;
pub use update::handle_update;
