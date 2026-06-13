use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct ExportCommand {
    /// Directory to export presets into. Defaults to the configured presets_dir.
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Overwrite existing files
    #[arg(long, short = 'f')]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct LinkCommand {
    /// Directory to use as the external presets source.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// Create the directory if it does not already exist.
    #[arg(long)]
    pub create: bool,
}

#[derive(Subcommand, Debug)]
pub enum OverlayCommands {
    /// Set the presets overlay directory in the active config.
    Link(LinkCommand),
    /// Remove the presets overlay directory from the active config.
    Unlink,
    /// Show the active presets overlay directory.
    Show,
}
