use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum StateCommands {
    /// Run maintenance checks now and report manual actions without applying them
    Check,
    /// Migrate and clean old shine-owned runtime state after schema changes
    Migrate(StateMigrateCommand),
}

#[derive(Args, Debug)]
pub struct StateMigrateCommand {
    /// Print migration steps without changing files
    #[arg(long)]
    pub dry_run: bool,
}
