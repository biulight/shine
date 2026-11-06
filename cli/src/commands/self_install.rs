use crate::update_check::ReleaseChannel;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SelfCommands {
    /// Copy the shine binary to a platform-specific install location
    Install {
        /// Destination path (default: /usr/local/bin/shine on Unix, %LOCALAPPDATA%\Programs\shine\shine.exe on Windows)
        #[arg(long, value_name = "PATH")]
        dest: Option<std::path::PathBuf>,
    },
    /// Remove this installation's managed resources, preserving personal data
    Uninstall {
        /// Preview removals without changing files
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed removals without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Download and install a shine release for this platform
    Upgrade {
        /// Release channel to install (default: stable)
        #[arg(long, value_enum)]
        channel: Option<ReleaseChannel>,
    },
}
