use std::path::PathBuf;

use clap::Args;

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
