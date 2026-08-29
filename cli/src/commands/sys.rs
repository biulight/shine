use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SysProfileCommands {
    /// Enable one item's Shine-managed shell integration without installing software
    Enable {
        #[arg(value_name = "ITEM")]
        item: String,
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed security Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Disable one item's Shine-managed shell integration without uninstalling software
    Disable {
        #[arg(value_name = "ITEM")]
        item: String,
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed security Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
}

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
    /// Bootstrap software and shell integration for the current OS
    Bootstrap {
        /// Bootstrap only these system items, in the given order
        #[arg(value_name = "ITEM", conflicts_with_all = ["preset", "exact_items"])]
        items: Vec<String>,
        /// Bootstrap one exact system item; repeat to preserve an explicit order
        #[arg(long = "item", value_name = "ITEM", action = clap::ArgAction::Append, conflicts_with_all = ["items", "preset"])]
        exact_items: Vec<String>,
        /// Apply a named profile without showing interactive selection
        #[arg(long, value_name = "PROFILE", conflicts_with = "items")]
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
        /// Approve the displayed security Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Manage Shine-owned shell integrations for bootstrap items
    Profile {
        #[command(subcommand)]
        command: SysProfileCommands,
    },
    /// Reapply enabled managed system configuration items
    Apply {
        /// Managed item to apply; applies all enabled items when omitted
        #[arg(value_name = "ITEM")]
        item: Option<String>,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed lifecycle Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Remove a managed system configuration item safely
    Uninstall {
        /// Managed item to remove
        #[arg(value_name = "ITEM")]
        item: String,
        /// Print what would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed lifecycle Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
}
