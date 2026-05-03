use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum EnvCommands {
    /// List all env variables
    Show,
    /// Set a variable in config.toml [env]
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
