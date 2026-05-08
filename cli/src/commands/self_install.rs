use crate::update_check::ReleaseChannel;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SelfCommands {
    /// Copy the shine binary to a system-wide location so `sudo shine` works
    Install {
        /// Destination path (default: /usr/local/bin/shine)
        #[arg(long, value_name = "PATH", default_value = "/usr/local/bin/shine")]
        dest: std::path::PathBuf,
    },
    /// Download and install a shine release for this platform
    Upgrade {
        /// Release channel to install (default: stable)
        #[arg(long, value_enum)]
        channel: Option<ReleaseChannel>,
    },
}
