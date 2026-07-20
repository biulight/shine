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

#[derive(Args, Debug)]
pub struct OverlayLinkCommand {
    /// Directory to use as the presets overlay. Mutually exclusive with --git.
    #[arg(value_name = "PATH", conflicts_with = "git")]
    pub path: Option<PathBuf>,
    /// Git URL for a shine-managed overlay. shine clones it (`--depth 1`) under
    /// `~/.shine/overlay` and keeps it mirrored to the remote tip on `shine pull`.
    #[arg(long, value_name = "URL")]
    pub git: Option<String>,
    /// Branch to track for --git. Defaults to the remote's default branch.
    #[arg(long, value_name = "BRANCH", requires = "git")]
    pub branch: Option<String>,
    /// Create the directory if it does not already exist (path mode only).
    #[arg(long)]
    pub create: bool,
}

#[derive(Subcommand, Debug)]
pub enum OverlayCommands {
    /// Set the presets overlay in the active config (local PATH or --git URL).
    Link(OverlayLinkCommand),
    /// Remove the presets overlay from the active config.
    Unlink,
    /// Show the active presets overlay.
    Show,
}
