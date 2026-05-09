use clap::{Args, Subcommand};

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
    /// Decode and decrypt a base64-encoded GPG secret from [env]
    Decrypt {
        /// Variable name containing base64-encoded GPG ciphertext
        key: String,
    },
    /// Encrypt stdin with GPG and print base64 ciphertext
    Encrypt(EnvEncryptCommand),
}

#[derive(Args, Debug)]
pub(crate) struct EnvEncryptCommand {
    /// GPG recipient key ID, fingerprint, or email
    #[arg(short = 'r', long)]
    pub recipient: String,
    /// Store the encrypted base64 value in config.toml [env] instead of printing it
    #[arg(long)]
    pub set: Option<String>,
    /// Read plaintext from an existing config.toml [env] variable instead of stdin
    #[arg(long)]
    pub from: Option<String>,
}
