use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum PresetTemplateKind {
    App,
    Shell,
}

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
pub struct CopyCommand {
    /// Built-in preset to copy (app/name, shell/name, or sys/name)
    #[arg(value_name = "KIND/NAME", value_parser = parse_copy_target)]
    pub target: String,
    /// Overwrite existing files
    #[arg(long, short = 'f')]
    pub force: bool,
}

pub(crate) fn parse_copy_target(value: &str) -> Result<String, String> {
    if value.contains('\\') {
        return Err(format!(
            "invalid preset target '{value}': expected app/name, shell/name, or sys/name"
        ));
    }

    let mut parts = value.split('/');
    let kind = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || !matches!(kind, "app" | "shell" | "sys")
        || name.is_empty()
        || matches!(name, "." | "..")
    {
        return Err(format!(
            "invalid preset target '{value}': expected app/name, shell/name, or sys/name"
        ));
    }
    Ok(value.to_string())
}

#[derive(Args, Debug)]
pub struct LinkCommand {
    /// Directory to use as the external presets source.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// Create the directory if it does not already exist.
    #[arg(long)]
    pub create: bool,
    /// Run external shell source changes on their next invocation.
    #[arg(long)]
    pub live: bool,
}

#[derive(Args, Debug)]
pub struct OverlayLinkCommand {
    /// Directory to use as the presets overlay. Mutually exclusive with --git.
    #[arg(value_name = "PATH", conflicts_with = "git")]
    pub path: Option<PathBuf>,
    /// Git URL for a shine-managed overlay. shine clones it (`--depth 1`) under
    /// `~/.shine/overlay` and keeps it mirrored to the remote tip on `shine preset pull`.
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
    /// Show information about the active presets overlay.
    Info,
}

#[derive(Subcommand, Debug)]
pub enum PresetCommands {
    /// Create a shine.toml template for a new app or shell preset
    New {
        #[arg(value_enum)]
        kind: PresetTemplateKind,
        /// Overwrite shine.toml if it already exists
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Copy built-in presets to a directory for local customization
    Export(ExportCommand),
    /// Copy one built-in preset into the current directory
    Copy(CopyCommand),
    /// Set the external presets directory in the active config
    Link(LinkCommand),
    /// Remove the external presets directory from the active config
    Unlink,
    /// Manage the personal presets overlay directory
    Overlay {
        #[command(subcommand)]
        command: OverlayCommands,
    },
    /// Pull Git-managed preset and overlay repositories
    Pull,
}
