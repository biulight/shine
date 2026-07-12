use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum ServeCommands {
    /// Install and start the local shine HTTP server as a user service
    Install(ServeInstallCommand),
    /// Start the local shine HTTP file server in the foreground
    Start(ServeStartCommand),
    /// Show whether the local shine HTTP user service is installed and loaded
    Status,
    /// Stop and remove the local shine HTTP user service
    Uninstall,
    /// Print the local URL for a managed HTTP resource
    Url(ServeUrlCommand),
}

#[derive(Args, Debug)]
pub struct ServeInstallCommand {
    /// Local port to listen on
    #[arg(long, default_value_t = 6174)]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct ServeStartCommand {
    /// Local port to listen on
    #[arg(long, default_value_t = 6174)]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct ServeUrlCommand {
    /// Resource path under ~/.shine/http, e.g. app/surge/custom-rules.sgmodule
    #[arg(value_name = "PATH")]
    pub path: String,
    /// Local port used by the shine HTTP server
    #[arg(long, default_value_t = 6174)]
    pub port: u16,
}
