use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AppCommands {
    /// List available app preset categories and their destination paths
    List,
    /// Show detailed information about a specific app preset category
    Info {
        /// Category to inspect (e.g. vim, starship)
        #[arg(value_name = "CATEGORY")]
        category: String,
    },
    /// Install app preset files for all or a specific category
    Install {
        /// Category to install (e.g. JetBrains, starship). Installs all if omitted.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Overwrite existing files even when content matches
        #[arg(long, short = 'f')]
        force: bool,
        /// Print what would be installed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Uninstall installed app preset files and optionally restore backups
    Uninstall {
        /// Also remove the app presets directory and manifest after uninstalling
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
}
