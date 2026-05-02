use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SysCommands {
    /// List available system init presets
    List,
    /// Run the system init script for the current OS
    Init {
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
    },
}
