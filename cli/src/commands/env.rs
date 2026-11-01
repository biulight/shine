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
    /// Set a variable in the selected env layer
    Set {
        /// Variable name (e.g. HTTP_PROXY_PORT)
        key: String,
        /// Variable value
        value: String,
        #[command(flatten)]
        target: crate::config::EnvWriteOptions,
    },
    /// Delete only the selected layer's definition, restoring inheritance
    Delete {
        key: String,
        #[command(flatten)]
        target: crate::config::EnvWriteOptions,
    },
    /// Explain configured sources without displaying values by default
    Explain {
        key: String,
        #[arg(long)]
        reveal: bool,
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
    /// Persist selected secret files at their mapped targets
    Files,
}

#[derive(Args, Debug)]
pub struct EnvWorkspaceExportCommand {
    /// Request file decryption through the local SSH broker
    #[arg(long, requires = "file_groups")]
    pub secret_broker: bool,
    /// Export format
    #[arg(
        long,
        value_enum,
        value_name = "FORMAT",
        requires_if("dotenv", "output"),
        requires_if("files", "file_groups")
    )]
    pub format: EnvWorkspaceExportFormat,
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Environment mode to resolve
    #[arg(long, value_name = "MODE")]
    pub mode: String,
    /// Destination dotenv file
    #[arg(long, value_name = "FILE", required_if_eq("format", "dotenv"))]
    pub output: Option<PathBuf>,
    /// Named secret file group to persist (repeatable; files format only)
    #[arg(
        long = "file-group",
        value_name = "NAME",
        required_if_eq("format", "files"),
        conflicts_with_all = ["output", "force", "include_secrets"]
    )]
    pub file_groups: Vec<String>,
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
        /// Describe a file-group release instead of environment variables
        #[arg(long = "file-group", conflicts_with_all = ["release", "release_all_declared"])]
        file_groups: Vec<String>,
        /// App category whose declared secret installation is being described
        #[arg(long, requires = "file_groups")]
        app: Option<String>,
        #[arg(long, value_parser = ["install", "upgrade", "export"], requires = "file_groups")]
        operation: Option<String>,
        #[arg(long, value_name = "FILE")]
        workspace: Option<PathBuf>,
        #[arg(long)]
        mode: String,
        #[arg(
            long,
            value_name = "KEY",
            required_unless_present_any = ["release_all_declared", "file_groups"],
            conflicts_with = "release_all_declared"
        )]
        release: Vec<String>,
        /// Release every secret declared by the selected source snapshot
        #[arg(long)]
        release_all_declared: bool,
        #[arg(
            required_unless_present = "file_groups",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
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
    #[arg(long = "file-group", conflicts_with_all = ["release", "release_all_declared"])]
    pub file_groups: Vec<String>,
    #[arg(long, requires = "file_groups")]
    pub app: Option<String>,
    #[arg(long, value_parser = ["install", "upgrade", "export"], requires = "file_groups")]
    pub operation: Option<String>,
    /// Explicitly permit upgrading the local broker policy store to version 2
    #[arg(long, requires = "file_groups")]
    pub migrate_file_policy: bool,

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
        required_unless_present_any = ["release_all_declared", "file_groups"],
        conflicts_with = "release_all_declared"
    )]
    pub release: Vec<String>,
    /// Release every secret declared by the selected source snapshot
    #[arg(long)]
    pub release_all_declared: bool,
    #[arg(
        required_unless_present = "file_groups",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
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
        #[arg(long = "file-group", conflicts_with_all = ["release", "release_all_declared"])]
        file_groups: Vec<String>,
        #[arg(long, requires = "file_groups")]
        app: Option<String>,
        #[arg(long, value_parser = ["install", "upgrade", "export"], requires = "file_groups")]
        operation: Option<String>,
        name: String,
        #[arg(long, value_name = "FILE")]
        workspace: PathBuf,
        #[arg(long)]
        mode: String,
        #[arg(
            long,
            value_name = "KEY",
            required_unless_present_any = ["release_all_declared", "file_groups"],
            conflicts_with = "release_all_declared"
        )]
        release: Vec<String>,
        /// Release every secret declared by the selected source snapshot
        #[arg(long)]
        release_all_declared: bool,
        #[arg(
            required_unless_present = "file_groups",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
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
    /// Seal workspace environment secrets or explicitly selected binary file groups
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
    /// Store ciphertext in the selected env layer instead of printing it
    #[arg(long)]
    pub set: Option<String>,
    /// Read plaintext from an effective Shine storage key instead of stdin
    #[arg(long)]
    pub from: Option<String>,
    #[command(flatten)]
    pub target: crate::config::EnvWriteOptions,
}

#[derive(Args, Debug)]
pub struct EnvSealCommand {
    /// Seal only this environment source file
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// Workspace definition (defaults to the nearest shine.workspace.toml)
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,
    /// Named workspace file group (repeatable); seals target plaintext into source ciphertext
    #[arg(long = "file-group", conflicts_with = "file")]
    pub file_groups: Vec<String>,
    /// File-group environment (defaults to env.default_mode)
    #[arg(long, requires = "file_groups")]
    pub mode: Option<String>,
    /// Verify all written ciphertext before removing the mapped plaintext files
    #[arg(long, requires = "file_groups")]
    pub remove_plaintext: bool,
    /// Preview file mappings without reading plaintext or invoking encryption
    #[arg(long, requires = "file_groups")]
    pub dry_run: bool,
    /// Secret backend: "gpg", "age", or workspace-only "hybrid"
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

/// Public recipient format requested from the phone plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum PhoneRecipientType {
    #[default]
    Tag,
    Phone,
}

impl PhoneRecipientType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Phone => "phone",
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
        /// Pair a phone-backed identity on Windows or macOS (experimental) and add its public stub to global Shine config
        #[arg(
            long,
            conflicts_with_all = ["touch_id", "access_control", "output", "force"]
        )]
        phone: bool,
        /// Phone recipient: tag (default, age 1.3+, plugin-free encryption) or phone
        #[arg(
            long,
            requires = "phone",
            conflicts_with_all = ["touch_id", "access_control", "output", "force"],
            value_enum,
            value_name = "TYPE"
        )]
        recipient_type: Option<PhoneRecipientType>,
        /// Desktop label shown during phone pairing (defaults to the computer name, or Shine desktop)
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
    /// Temporarily decrypt a named workspace file group (repeatable)
    #[arg(long = "file-group", conflicts_with_all = ["no_workspace", "secret_broker"])]
    pub file_groups: Vec<String>,
    /// Preview file mappings without decryption or command execution
    #[arg(long, requires = "file_groups")]
    pub dry_run: bool,
    /// Command and arguments to run
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}

#[cfg(test)]
mod phone_recipient_tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct IdentityCli {
        #[command(flatten)]
        identity: EnvIdentityCommand,
    }

    #[test]
    fn phone_recipient_cli_defaults_and_explicit_types() {
        for (extra, expected) in [
            (vec![], PhoneRecipientType::Tag),
            (vec!["--recipient-type", "tag"], PhoneRecipientType::Tag),
            (vec!["--recipient-type", "phone"], PhoneRecipientType::Phone),
        ] {
            let mut args = vec!["identity", "init", "--phone"];
            args.extend(extra);
            let parsed = IdentityCli::try_parse_from(args).unwrap();
            let EnvIdentitySubcommand::Init { recipient_type, .. } = parsed.identity.command else {
                panic!("expected init")
            };
            assert_eq!(recipient_type.unwrap_or_default(), expected);
        }
    }

    #[test]
    fn phone_recipient_cli_rejects_invalid_combinations() {
        for args in [
            vec!["init", "--recipient-type", "tag"],
            vec!["init", "--phone", "--recipient-type", "unknown"],
            vec!["init", "--touch-id", "--recipient-type", "tag"],
            vec![
                "init",
                "--recipient-type",
                "tag",
                "--output",
                "identity.txt",
            ],
            vec!["init", "--recipient-type", "tag", "--force"],
            vec![
                "init",
                "--recipient-type",
                "tag",
                "--access-control",
                "passcode",
            ],
            vec!["init", "--phone", "--touch-id"],
            vec!["init", "--phone", "--output", "identity.txt"],
            vec!["init", "--phone", "--force"],
            vec!["init", "--phone", "--access-control", "passcode"],
        ] {
            assert!(
                IdentityCli::try_parse_from(
                    std::iter::once("identity").chain(args.iter().copied())
                )
                .is_err(),
                "accepted {args:?}"
            );
        }
    }
}

#[cfg(test)]
mod workspace_export_tests {
    use crate::commands::Cli;
    use clap::Parser;

    #[test]
    fn export_format_argument_contract() {
        let prefix = ["shine", "env", "workspace", "export"];
        for args in [
            "--format dotenv --mode prod --output out.env",
            "--format dotenv --mode prod --output out.env --force --include-secrets",
            "--format files --mode prod --file-group signing --file-group account --dry-run",
        ] {
            Cli::try_parse_from(prefix.into_iter().chain(args.split_whitespace())).unwrap();
        }
        for args in [
            "--format dotenv --mode prod",
            "--format dotenv --mode prod --file-group signing",
            "--format dotenv --mode prod --output out.env --file-group signing",
            "--format files --mode prod",
            "--format files --mode prod --output out.env",
            "--format files --file-group signing",
            "--format files --mode prod --file-group signing --output out.env",
            "--format files --mode prod --file-group signing --force",
            "--format files --mode prod --file-group signing --include-secrets",
        ] {
            assert!(
                Cli::try_parse_from(prefix.into_iter().chain(args.split_whitespace())).is_err(),
                "{args}"
            );
        }
    }
}
