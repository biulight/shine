use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum EnvCommands {
    /// Show the path of env.toml
    Path,
    /// List all env variables
    Show,
    /// Set a variable (creates env.toml if needed)
    Set {
        /// Variable name (e.g. HTTP_PROXY_PORT)
        key: String,
        /// Variable value
        value: String,
    },
    /// Get a single variable value
    Get {
        /// Variable name
        key: String,
    },
}
