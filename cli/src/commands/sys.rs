use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SysCommands {
    /// List available system init presets
    List,
    /// Run the system init script for the current OS
    Init {
        /// Apply a named profile without showing interactive selection
        #[arg(long, value_name = "PROFILE")]
        preset: Option<String>,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
    },
}
