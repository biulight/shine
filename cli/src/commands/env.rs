use clap::{Args, Subcommand};
use std::{ffi::OsString, path::PathBuf};

#[derive(Subcommand, Debug)]
pub enum EnvCommands {
    /// List all env variables
    Show {
        /// Show sensitive values instead of redacting them
        #[arg(long)]
        reveal: bool,
    },
    /// Set a variable in config.toml [env]
    Set {
        /// Variable name (e.g. HTTP_PROXY_PORT)
        key: String,
        /// Variable value
        value: String,
    },
    /// Delete a variable from config.toml [env]
    Delete {
        /// Variable name
        key: String,
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
    /// Decrypt KEY_SECRET and print shell code that exports KEY
    Export {
        /// Variable name to export from KEY_SECRET
        key: String,
        /// Export under a different name in the current shell
        #[arg(long = "as", value_name = "ALIAS")]
        alias: Option<String>,
    },
    /// Encrypt stdin with GPG and print base64 ciphertext
    Encrypt(EnvEncryptCommand),
    /// Seal pending secrets in workspace environment files
    Seal(EnvSealCommand),
    /// Run a command with the workspace environment
    Run(EnvRunCommand),
}

#[derive(Args, Debug)]
pub struct EnvEncryptCommand {
    /// GPG recipient key ID, fingerprint, or email
    #[arg(short = 'r', long)]
    pub recipient: Option<String>,
    /// Store the encrypted base64 value in config.toml [env] instead of printing it
    #[arg(long)]
    pub set: Option<String>,
    /// Read plaintext from an existing config.toml [env] variable instead of stdin
    #[arg(long)]
    pub from: Option<String>,
}

#[derive(Args, Debug)]
pub struct EnvSealCommand {
    /// Seal only this environment source file
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// GPG recipient key ID, fingerprint, or email
    #[arg(short = 'r', long)]
    pub recipient: Option<String>,
}

#[derive(Args, Debug)]
pub struct EnvRunCommand {
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Environment mode used to expand {mode} paths
    #[arg(long)]
    pub mode: Option<String>,
    /// Command and arguments to run
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}
