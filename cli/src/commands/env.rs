use clap::{Args, Subcommand, ValueEnum};
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
    /// Run a command with the workspace environment
    Run(EnvRunCommand),
    /// Create and manage workspace environment definitions
    Workspace(EnvWorkspaceCommand),
    /// Transparently proxy selected commands with explicitly injected values
    Proxy(EnvProxyCommand),
    /// Manage SSH secret-broker policies and describe workspace requests
    Broker(EnvBrokerCommand),
    /// Encrypt, decrypt, export, and manage secret identities
    Secret(EnvSecretCommand),
}

#[derive(Args, Debug)]
pub struct EnvWorkspaceCommand {
    #[command(subcommand)]
    pub command: EnvWorkspaceSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum EnvWorkspaceSubcommand {
    /// Create a workspace from conventional dotenv files
    Init(EnvWorkspaceInitCommand),
    /// Export one resolved workspace mode without retaining a Shine dependency
    Export(EnvWorkspaceExportCommand),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum EnvWorkspaceExportFormat {
    /// Conventional KEY=VALUE dotenv output
    Dotenv,
}

#[derive(Args, Debug)]
pub struct EnvWorkspaceExportCommand {
    /// Export format
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub format: EnvWorkspaceExportFormat,
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Environment mode to resolve
    #[arg(long, value_name = "MODE")]
    pub mode: String,
    /// Destination dotenv file
    #[arg(long, value_name = "FILE")]
    pub output: PathBuf,
    /// Decrypt and include sealed workspace secrets
    #[arg(long)]
    pub include_secrets: bool,
    /// Replace an existing output file
    #[arg(long)]
    pub force: bool,
    /// Validate and describe the export without writing it
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct EnvWorkspaceInitCommand {
    /// Import .env, .env.local, .env.<mode>, and .env.<mode>.local files
    #[arg(long)]
    pub from_dotenv: bool,
    /// Mode to import (repeatable); modes are discovered when omitted
    #[arg(long, value_name = "MODE")]
    pub mode: Vec<String>,
    /// Import this key as an encrypted workspace secret (repeatable)
    #[arg(long, value_name = "KEY")]
    pub secret: Vec<String>,
    /// Replace generated workspace files that already exist
    #[arg(long)]
    pub force: bool,
    /// Print planned files without writing them
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct EnvProxyCommand {
    #[command(subcommand)]
    pub command: EnvProxySubcommand,
}

#[derive(Args, Debug)]
pub struct EnvBrokerCommand {
    #[command(subcommand)]
    pub command: EnvBrokerSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum EnvBrokerSubcommand {
    /// Describe a workspace request without decrypting or running its command
    Describe {
        #[arg(long, value_name = "FILE")]
        workspace: Option<PathBuf>,
        #[arg(long)]
        mode: String,
        #[arg(
            long,
            value_name = "KEY",
            required_unless_present = "release_all_declared",
            conflicts_with = "release_all_declared"
        )]
        release: Vec<String>,
        /// Release every secret declared by the selected source snapshot
        #[arg(long)]
        release_all_declared: bool,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Manage local SSH secret-broker authorization policies
    Policy(EnvBrokerPolicyCommand),
}

#[derive(Args, Debug)]
pub struct EnvBrokerPolicyCommand {
    #[command(subcommand)]
    pub command: EnvBrokerPolicySubcommand,
}

#[derive(Args, Debug)]
pub struct EnvBrokerPolicyInput {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub ssh_target: String,
    #[arg(long, default_value = "")]
    pub project: String,
    #[arg(long, value_name = "FILE")]
    pub workspace: PathBuf,
    /// Optionally require the remote workspace file to have this exact path
    #[arg(long, value_name = "REMOTE_FILE")]
    pub remote_workspace: Option<String>,
    #[arg(long)]
    pub mode: String,
    #[arg(
        long,
        value_name = "KEY",
        required_unless_present = "release_all_declared",
        conflicts_with = "release_all_declared"
    )]
    pub release: Vec<String>,
    /// Release every secret declared by the selected source snapshot
    #[arg(long)]
    pub release_all_declared: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum EnvBrokerPolicySubcommand {
    /// Add a policy generated from a trusted local workspace checkout
    Add(EnvBrokerPolicyInput),
    /// Replace a policy from a trusted local workspace checkout
    Update(EnvBrokerPolicyInput),
    /// Show whether a trusted local workspace still matches a policy
    Diff {
        name: String,
        #[arg(long, value_name = "FILE")]
        workspace: PathBuf,
        #[arg(long)]
        mode: String,
        #[arg(
            long,
            value_name = "KEY",
            required_unless_present = "release_all_declared",
            conflicts_with = "release_all_declared"
        )]
        release: Vec<String>,
        /// Release every secret declared by the selected source snapshot
        #[arg(long)]
        release_all_declared: bool,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// List configured policies
    List,
    /// Print one policy
    Info { name: String },
    /// Remove one policy
    Remove { name: String },
}

#[derive(Subcommand, Debug)]
pub enum EnvProxySubcommand {
    /// Install a PATH shim and configure its allowed environment values
    Install {
        #[arg(value_name = "COMMAND")]
        command: String,
        #[arg(long = "with", value_name = "KEY[=ALIAS]", required = true)]
        with: Vec<String>,
        /// Store the rule in the current project's shine.config.toml
        #[arg(long)]
        project: bool,
    },
    /// List installed transparent command proxies
    List,
    /// Remove a shine-managed command proxy and its user-level rule
    Uninstall {
        #[arg(value_name = "COMMAND")]
        command: String,
    },
    /// Enable secret injection for an installed command proxy
    Enable {
        #[arg(value_name = "COMMAND")]
        command: String,
        /// Change the rule in the current project's shine.config.toml
        #[arg(long)]
        project: bool,
    },
    /// Bypass secret injection while retaining the installed command proxy
    Disable {
        #[arg(value_name = "COMMAND")]
        command: String,
        /// Change the rule in the current project's shine.config.toml
        #[arg(long)]
        project: bool,
    },
    #[command(hide = true)]
    Exec {
        #[arg(long)]
        target: PathBuf,
        #[arg(value_name = "COMMAND")]
        command: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

#[derive(Args, Debug)]
pub struct EnvSecretCommand {
    #[command(subcommand)]
    pub command: EnvSecretSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum EnvSecretSubcommand {
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

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum PhoneIdentityTransport {
    #[default]
    Auto,
    Adb,
    Qr,
}

impl PhoneIdentityTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Adb => "adb",
            Self::Qr => "qr",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum EnvIdentitySubcommand {
    /// Generate a new age identity, optionally backed by Touch ID or a paired phone
    Init {
        /// Generate a Secure Enclave identity requiring Touch ID (macOS only)
        #[arg(long, conflicts_with = "phone")]
        touch_id: bool,
        /// Pair a phone-backed identity and add its public stub to global Shine config
        #[arg(
            long,
            conflicts_with_all = ["touch_id", "access_control", "output", "force"]
        )]
        phone: bool,
        /// Desktop label shown during phone pairing (defaults to the Windows computer name)
        #[arg(long, requires = "phone", value_name = "LABEL")]
        label: Option<String>,
        /// Phone pairing transport: auto, adb, or qr
        #[arg(long, requires = "phone", value_enum, value_name = "TRANSPORT")]
        transport: Option<PhoneIdentityTransport>,
        /// Explicit ADB device serial when more than one device is online
        #[arg(long, requires = "phone", value_name = "SERIAL")]
        adb_serial: Option<String>,
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
    /// Request secrets from the local end of the current shine ssh session
    #[arg(long)]
    pub secret_broker: bool,
    /// Request one session-authorized encrypted key as KEY or KEY=ALIAS
    #[arg(
        long = "secret",
        value_name = "KEY[=ALIAS]",
        requires = "secret_broker",
        requires = "no_workspace"
    )]
    pub secret: Vec<String>,
    /// Command and arguments to run
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}
