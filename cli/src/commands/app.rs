use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AppCommands {
    /// Generate an app preset shine.toml template in the current directory
    Init {
        /// Overwrite shine.toml if it already exists
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// List available app preset categories and their destination paths
    List,
    /// Show detailed information about a specific app preset category
    Info {
        /// Category to inspect (e.g. vim, starship)
        #[arg(value_name = "CATEGORY")]
        category: String,
    },
    /// Install app preset files for all or a specific category
    Install {
        /// Category to install (e.g. JetBrains, starship). Installs all if omitted.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Print what would be installed without making any changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Reinstall app preset files for all or a specific category, overwriting managed files
    Reinstall {
        /// Category to reinstall (e.g. JetBrains, starship). Reinstalls all if omitted.
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Print what would be installed without making any changes
        #[arg(long)]
        dry_run: bool,
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
    },
    /// Run the artifact build script declared by an app preset
    Build {
        /// App preset category whose artifact script should run (e.g. surge)
        #[arg(value_name = "APP_ID")]
        app_id: String,
    },
    /// Run the artifact teardown script declared by an app preset (reverses `build`)
    Unbuild {
        /// App preset category whose artifact teardown script should run (e.g. surge)
        #[arg(value_name = "APP_ID")]
        app_id: String,
    },
}
