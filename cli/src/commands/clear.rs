use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ClearCommands {
    /// Stop and remove the retired local HTTP service, preserving published files
    Serve,
}
