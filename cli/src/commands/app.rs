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
        /// Explicitly execute generators to evaluate final transformed content
        #[arg(long)]
        run_generators: bool,
        /// Print a unified diff against installed content (or an empty file before install)
        #[arg(long)]
        diff: bool,
    },
    /// Install app preset files for all or a specific category
    Install {
        /// Category to install (e.g. JetBrains, starship). Installs all if omitted.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Print what would be installed without making any changes
        #[arg(long)]
        dry_run: bool,
        /// Replace user-modified files that are already managed by shine
        #[arg(long)]
        replace_managed: bool,
        /// Approve the displayed lifecycle Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Explicitly refresh installed generated files for an app preset
    Refresh {
        /// App preset category to refresh
        #[arg(value_name = "CATEGORY")]
        category: String,
        /// Optional generator source path; refreshes all installed generators when omitted
        #[arg(value_name = "FILE")]
        file: Option<String>,
        /// Overwrite a managed destination that was modified after install
        #[arg(long)]
        force: bool,
        /// Approve the displayed security Plan without prompting
        #[arg(long)]
        yes: bool,
    },
    /// Review and recover an interrupted app lifecycle operation
    Recover {
        /// Approve the displayed recovery Plan without prompting
        #[arg(long)]
        yes: bool,
    },
    /// Uninstall installed app preset files and optionally restore backups
    Uninstall {
        /// Category to uninstall (e.g. vim, starship). Uninstalls all if omitted.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Remove app config files even if they were modified after install
        #[arg(long)]
        force: bool,
        /// Also remove the app presets directory and manifest after uninstalling
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without making any changes
        #[arg(long)]
        dry_run: bool,
        /// Approve the displayed lifecycle Plan without prompting
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
    },
    /// Apply or remove an app preset's external artifact integration
    Artifact {
        #[command(subcommand)]
        command: AppArtifactCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum AppArtifactCommands {
    /// Apply the artifact integration declared by an app preset
    Apply {
        #[arg(value_name = "APP_ID")]
        app_id: String,
        /// Approve the displayed security Plan without prompting
        #[arg(long)]
        yes: bool,
    },
    /// Remove the artifact integration declared by an app preset
    Remove {
        #[arg(value_name = "APP_ID")]
        app_id: String,
        /// Approve the displayed security Plan without prompting
        #[arg(long)]
        yes: bool,
    },
}
