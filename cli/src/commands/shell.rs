use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ShellCommands {
    /// Generate a shell preset shine.toml template in the current directory
    Init {
        /// Overwrite shine.toml if it already exists
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// List available shell preset categories and their scripts
    List,
    /// Install shell presets and create bin symlinks.
    /// Run 'shine shell list' to see available categories.
    Install {
        /// Preset category to install (e.g. "proxy"). Installs all if omitted.
        /// Run 'shine shell list' to see available categories.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Overwrite existing preset files, symlinks, and shell config entry
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Uninstall shell presets and remove bin symlinks.
    /// Run 'shine shell list' to see installed categories.
    Uninstall {
        /// Preset category to uninstall (e.g. "proxy"). Uninstalls all if omitted.
        /// Run 'shine shell list' to see installed categories.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Also remove empty managed directories after uninstall
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
}
