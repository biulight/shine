use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ShellCommands {
    /// List all shells
    List,
    /// Install all shell presets and create bin symlinks
    Install,
    /// Uninstall shell presets and remove bin symlinks
    Uninstall {
        /// Also remove empty managed directories after uninstall
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Proxy
    Proxy,
}
