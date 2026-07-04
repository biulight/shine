use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SysCommands {
    /// List available system init presets
    List,
    /// Show system init items previously initialized by shine
    Status,
    /// Run the system init script for the current OS
    Init {
        /// Apply a named profile without showing interactive selection
        #[arg(long, value_name = "PROFILE")]
        preset: Option<String>,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Back up and replace the sys profile instead of merging user edits
        #[arg(long)]
        force_profile: bool,
    },
    /// Reapply enabled managed system configuration items
    Apply {
        /// Managed item to apply; applies all enabled items when omitted
        #[arg(value_name = "ITEM")]
        item: Option<String>,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a managed system configuration item safely
    Uninstall {
        /// Managed item to remove
        #[arg(value_name = "ITEM")]
        item: String,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
    },
}
