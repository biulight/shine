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
}
