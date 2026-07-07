use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum LocalCommands {
    /// Download a file from the remote host to the local machine
    Download(LocalTransferCommand),
    /// Upload a file from the local machine to the remote host
    Upload(LocalTransferCommand),
}

#[derive(Args, Debug)]
pub struct LocalTransferCommand {
    /// Source path, resolved by whichever side owns it (remote for
    /// download, local for upload)
    pub source: String,
    /// Destination path, resolved by the other side; defaults to the same
    /// file name in that side's default directory
    pub destination: Option<String>,
    /// Overwrite an existing destination
    #[arg(long)]
    pub force: bool,
    /// Show what would happen without transferring any data
    #[arg(long)]
    pub dry_run: bool,
}
