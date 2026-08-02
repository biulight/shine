use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SysCommands {
    /// List available system items
    List {
        /// Show items for every supported operating system
        #[arg(long)]
        all: bool,
    },
    /// Show detailed information about a system item
    Info {
        /// System item to inspect
        #[arg(value_name = "ITEM")]
        item: String,
    },
    /// Show system bootstrap items previously initialized by shine
    Status,
    /// Check recorded bootstrap software for updates without upgrading it
    Update {
        /// Recorded bootstrap item to check
        #[arg(value_name = "ITEM")]
        item: Option<String>,
        /// Also show current and manual-check-only items
        #[arg(long)]
        verbose: bool,
        /// Route package-manager update checks through shine's preset proxy
        #[arg(long)]
        proxy: bool,
    },
    /// Bootstrap software and shell integration for the current OS
    Bootstrap {
        /// Apply a named profile without showing interactive selection
        #[arg(long, value_name = "PROFILE")]
        preset: Option<String>,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Back up and replace the sys profile instead of merging user edits
        #[arg(long)]
        force_profile: bool,
        /// Route init-script downloads through shine's preset proxy ([env] PROXY_HOST/HTTP_PROXY_PORT)
        #[arg(long)]
        proxy: bool,
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
