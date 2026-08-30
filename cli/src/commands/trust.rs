use clap::Subcommand;

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum TrustCommands {
    /// List current external-code trust grants
    List,
    /// Inspect the current external-code requirements for one canonical target
    Inspect {
        #[arg(value_name = "TARGET")]
        target: String,
    },
    /// Trust the current external code and declared permissions for one target
    Grant {
        #[arg(value_name = "TARGET")]
        target: String,
        /// Confirm the rendered trust scope without prompting
        #[arg(long)]
        yes: bool,
    },
    /// Revoke every external-code grant for one target
    Revoke {
        #[arg(value_name = "TARGET")]
        target: String,
    },
}
