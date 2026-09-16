use clap::Subcommand;

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum TrustCommands {
    /// List current external-code trust grants
    List,
    /// Inspect one canonical target or every current target with `preset`
    Inspect {
        #[arg(value_name = "TARGET")]
        target: String,
    },
    /// Trust one canonical target or every current target with `preset`
    Grant {
        #[arg(value_name = "TARGET")]
        target: String,
        /// Confirm the rendered trust scope without prompting
        #[arg(long)]
        yes: bool,
    },
    /// Revoke one target, or every Preset grant with `preset`
    Revoke {
        #[arg(value_name = "TARGET")]
        target: String,
    },
}
