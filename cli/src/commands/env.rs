use clap::{Args, Subcommand};
use std::{ffi::OsString, path::PathBuf};

#[derive(Subcommand, Debug)]
pub enum EnvCommands {
    /// List all env variables
    List {
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
        /// Write directly into the env override file that currently shadows this
        /// key (global/overlay/project shine.env.toml) instead of refusing
        #[arg(long)]
        force: bool,
    },
    /// Delete a variable from config.toml [env]
    Delete {
        /// Variable name
        key: String,
        /// Delete directly from the env override file that currently shadows
        /// this key (global/overlay/project shine.env.toml) instead of refusing
        #[arg(long)]
        force: bool,
    },
    /// Get a single variable value
    Get {
        /// Variable name
        key: String,
    },
    /// Decode and decrypt an encrypted secret from [env] (GPG or age)
    Decrypt {
        /// Variable name containing encrypted ciphertext
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
    /// Encrypt stdin and print ciphertext (GPG by default, or age with --backend age)
    Encrypt(EnvEncryptCommand),
    /// Seal pending secrets in workspace environment files
    Seal(EnvSealCommand),
    /// Run a command with the workspace environment
    Run(EnvRunCommand),
    /// Manage age identities used to decrypt age-backed secrets
    Identity(EnvIdentityCommand),
}

#[derive(Args, Debug)]
pub struct EnvEncryptCommand {
    /// Secret backend to use: "gpg" (default) or "age"
    #[arg(long)]
    pub backend: Option<String>,
    /// Recipient (repeatable): GPG key ID/fingerprint/email, or age recipient
    #[arg(short = 'r', long = "recipient")]
    pub recipients: Vec<String>,
    /// Store the encrypted ciphertext in config.toml [env] instead of printing it
    #[arg(long)]
    pub set: Option<String>,
    /// Read plaintext from an existing config.toml [env] variable instead of stdin
    #[arg(long)]
    pub from: Option<String>,
    /// Write directly into the env override file that currently shadows the
    /// target key (global/overlay/project shine.env.toml) instead of refusing
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct EnvSealCommand {
    /// Seal only this environment source file
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Secret backend to use: "gpg" (default) or "age"
    #[arg(long)]
    pub backend: Option<String>,
    /// Recipient (repeatable): GPG key ID/fingerprint/email, or age recipient
    #[arg(short = 'r', long = "recipient")]
    pub recipients: Vec<String>,
}

#[derive(Args, Debug)]
pub struct EnvIdentityCommand {
    #[command(subcommand)]
    pub command: EnvIdentitySubcommand,
}

#[derive(Subcommand, Debug)]
pub enum EnvIdentitySubcommand {
    /// Generate a new age identity, optionally backed by Touch ID (Secure Enclave)
    Init {
        /// Generate a Secure Enclave identity requiring Touch ID (macOS only)
        #[arg(long)]
        touch_id: bool,
        /// Secure Enclave access control policy (only with --touch-id): any-biometry
        /// (default), any-biometry-or-passcode, current-biometry, or passcode
        #[arg(long, value_name = "POLICY")]
        access_control: Option<String>,
        /// Output path (defaults to <shine_dir>/age/identity.txt)
        #[arg(short = 'o', long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Overwrite an existing identity file
        #[arg(long)]
        force: bool,
    },
    /// Print the recipient(s) for the configured identity file(s)
    List,
}

#[derive(Args, Debug)]
pub struct EnvRunCommand {
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Environment mode used to expand {mode} paths
    #[arg(long)]
    pub mode: Option<String>,
    /// Skip workspace discovery entirely; use only --with values and inherited env
    #[arg(long, conflicts_with_all = ["workspace", "mode"])]
    pub no_workspace: bool,
    /// Inject a config [env] value as KEY or KEY=ALIAS (repeatable)
    #[arg(long = "with", value_name = "KEY[=ALIAS]")]
    pub with: Vec<String>,
    /// Command and arguments to run
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}
