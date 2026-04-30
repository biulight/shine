use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PresetsCommands {
    /// Copy all built-in embedded presets to the presets directory for local customization.
    /// Defaults to the currently configured presets directory (~/.shine/presets/).
    Export {
        /// Directory to export presets into. Defaults to the configured presets_dir.
        #[arg(value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Overwrite existing files
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Set the external presets directory in ~/.shine/config.toml.
    /// After linking, all install/check/list commands read presets from PATH instead of
    /// the embedded binary. Run `shine presets export` to seed the directory with built-ins.
    Link {
        /// Directory to use as the external presets source.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Create the directory if it does not already exist.
        #[arg(long)]
        create: bool,
    },
    /// Remove the external presets directory from ~/.shine/config.toml,
    /// reverting to the built-in embedded presets.
    Unlink,
}
