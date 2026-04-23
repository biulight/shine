use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ShellCommands {
    /// List available shell preset categories and their scripts
    List,
    /// Install shell presets and create bin symlinks.
    /// Run 'shine shell list' to see available categories.
    Install {
        /// Preset category to install (e.g. "proxy"). Installs all if omitted.
        /// Run 'shine shell list' to see available categories.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
    },
    /// Uninstall shell presets and remove bin symlinks
    Uninstall {
        /// Also remove empty managed directories after uninstall
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
}
