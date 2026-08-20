use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ShellCommands {
    /// List available shell preset categories and their scripts
    List,
    /// Show detailed information about a shell preset category or command
    Info {
        /// Category, command, or category/command to inspect
        #[arg(value_name = "TARGET")]
        target: String,
    },
    /// Install shell presets and create bin symlinks.
    /// Run 'shine shell list' to see available categories.
    Install {
        /// Category or category/command to install (e.g. "proxy" or "utils/shine-env-export"). Installs all if omitted.
        /// Run 'shine shell list' to see available categories.
        #[arg(value_name = "TARGET")]
        target: Option<String>,
        /// Replace user-modified managed files, links, and profile integration
        #[arg(long)]
        replace_managed: bool,
    },
    /// Uninstall shell presets and remove bin symlinks.
    /// Run 'shine shell list' to see installed categories.
    Uninstall {
        /// Category or category/command to uninstall. Uninstalls all if omitted.
        /// Run 'shine shell list' to see installed categories.
        #[arg(value_name = "TARGET")]
        target: Option<String>,
        /// Also remove empty managed directories after uninstall
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
}
