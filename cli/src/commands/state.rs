use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum StateCommands {
    /// Migrate and clean old shine-owned runtime state after schema changes
    Migrate(StateMigrateCommand),
}

#[derive(Args, Debug)]
pub struct StateMigrateCommand {
    /// Print migration steps without changing files
    #[arg(long)]
    pub dry_run: bool,
}
