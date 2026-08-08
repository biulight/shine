use crate::completion;
use crate::version;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use super::{
    AppCommands, EnvCommands, LocalCommands, PresetCommands, SelfCommands, ServeCommands,
    ShellCommands, StateCommands, SysCommands, TaskCommands, TaskRunCommand, ThemeCommands,
};

/// Manage shell presets, app configs, system setup, and personal tools
#[derive(Parser, Debug)]
#[command(name = "shine")]
#[command(version = version::display(), about, long_about = None)]
#[command(
    after_help = "QUICK START:\n  shine list --available\n  shine info app/starship\n  shine install app/starship\n  shine update && shine upgrade\n\nTARGETS:\n  Use app/<category>, shell/<category>, or sys/<item>. A bare app/shell category is accepted when unique.\n\nNAMESPACES:\n  app, shell, and sys expose resource-specific operations; preset, state, self, serve, completions, theme, and local are advanced tools."
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(name = "__shell-render", hide = true)]
    ShellRender {
        #[arg(value_name = "TARGET")]
        target: String,
    },
    /// Initialize the current directory as a shine presets directory
    Init(InitCommand),
    /// Manage shell command presets
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },
    /// Manage application configuration presets
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    /// Install or repair one shell or app preset
    Install {
        /// Preset target: app/<category>, shell/<category>, or a unique category name
        #[arg(value_name = "TARGET")]
        target: String,
        /// Replace user-modified files that are already managed by shine
        #[arg(long)]
        replace_managed: bool,
    },
    /// Uninstall one shell or app preset
    Uninstall {
        /// Preset target: app/<category>, shell/<category>, or a unique category name
        #[arg(value_name = "TARGET")]
        target: String,
        /// Remove managed files even when they were modified after installation (app only)
        #[arg(long)]
        force: bool,
        /// Also remove empty managed preset directories
        #[arg(long)]
        purge: bool,
        /// Print what would be removed without changing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate or install shell completion scripts
    Completions {
        #[command(subcommand)]
        command: CompletionCommands,
    },
    /// List installed resources, or browse available resources with --available
    List {
        /// List available resources instead of installed resources
        #[arg(long)]
        available: bool,
        /// Limit --available output to app, shell, or sys resources
        #[arg(value_enum, requires = "available", value_name = "KIND")]
        kind: Option<ResourceKind>,
    },
    /// Show details for an available or installed app/shell target, or `sys/<ITEM>`
    Info {
        /// Installed item to inspect (e.g. git, starship, proxy, setproxy)
        #[arg(value_name = "TARGET")]
        target: String,
        /// Also print a unified diff against the expected content
        #[arg(long)]
        diff: bool,
        /// Also print the installed or rendered file content
        #[arg(long)]
        verbose: bool,
    },
    /// Manage preset sources, overlays, exports, and Git synchronization
    Preset {
        #[command(subcommand)]
        command: PresetCommands,
    },
    /// Check managed configuration and shine release updates
    Update(UpdateCommand),
    /// Apply available managed configuration updates
    Upgrade(UpgradeCommand),
    /// Manage shine-owned runtime state
    State {
        #[command(subcommand)]
        command: StateCommands,
    },
    /// Manage the shine binary itself
    #[command(name = "self")]
    Self_ {
        #[command(subcommand)]
        command: SelfCommands,
    },
    /// Serve shine-managed HTTP resources from ~/.shine/http
    Serve {
        #[command(subcommand)]
        command: ServeCommands,
    },
    /// Manage preset variables and workspace command environments
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
    /// Manage system bootstrap and configuration for the current OS
    Sys {
        #[command(subcommand)]
        command: SysCommands,
    },
    /// Resolve and sync the terminal's light/dark theme (see `shine theme sync`)
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Open an interactive SSH session with a session-scoped file transfer channel
    Ssh {
        /// Remote command shell (must appear before the SSH destination).
        /// Windows mode injects environment variables only; `shine local` is unavailable.
        #[arg(long, value_enum, default_value_t = RemoteShell::Posix)]
        remote_shell: RemoteShell,
        /// Inject a plaintext config [env] value as KEY or KEY=ALIAS (repeatable;
        /// must appear before the SSH destination)
        #[arg(long = "with", value_name = "KEY[=ALIAS]")]
        with: Vec<String>,
        /// Decrypt KEY_SECRET and inject it as KEY or ALIAS (repeatable; must
        /// appear before the SSH destination)
        #[arg(long = "with-secret", value_name = "KEY[=ALIAS]")]
        with_secret: Vec<String>,
        /// Enable the session-scoped, on-demand secret broker
        #[arg(long)]
        secret_broker: bool,
        /// Merge an additional local broker policy file (repeatable). The same
        /// ownership, permission, and symlink checks apply.
        #[arg(
            long = "secret-broker-policy",
            value_name = "FILE",
            requires = "secret_broker"
        )]
        secret_broker_policy: Vec<PathBuf>,
        /// Allow one encrypted local config key to be requested by a direct
        /// broker command (repeatable; requires local confirmation per request)
        #[arg(
            long = "allow-secret",
            value_name = "KEY[=ALIAS]",
            requires = "secret_broker"
        )]
        allow_secret: Vec<String>,
        /// Trust the entire remote session and auto-approve matching workspace
        /// policies. Never applies to direct --allow-secret requests.
        #[arg(long, requires = "secret_broker")]
        trust_remote_session: bool,
        /// Inspect one remote workspace broker description without writing a
        /// policy or releasing secrets
        #[arg(long, conflicts_with_all = ["secret_broker", "secret_broker_enroll"])]
        secret_broker_inspect: bool,
        /// Enroll one policy from explicitly trusted remote metadata; never
        /// decrypts or runs the described command
        #[arg(long, conflicts_with_all = ["secret_broker", "secret_broker_inspect"])]
        secret_broker_enroll: bool,
        /// Required acknowledgement that enrollment trusts remote metadata
        #[arg(long, requires = "secret_broker_enroll")]
        trust_remote_metadata: bool,
        /// ssh options, the destination, and an optional remote command
        /// (passed through to the system `ssh` binary; see `ssh(1)`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Transfer files between this machine and the other end of a `shine ssh` session
    Local {
        #[command(subcommand)]
        command: LocalCommands,
    },
    /// Save, run, and manage personal shortcut commands
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    /// Run a saved task (alias for `shine task run`)
    Run(TaskRunCommand),
}

/// Shell used by the remote SSH server to interpret Shine's command wrapper.
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum RemoteShell {
    /// POSIX shell with session-scoped `shine local` file transfer support.
    Posix,
    /// Windows PowerShell environment injection only; `shine local` is unavailable.
    Windows,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ResourceKind {
    App,
    Shell,
    Sys,
}

#[derive(Args, Debug)]
pub struct InitCommand {
    /// Skip the confirmation prompt
    #[arg(long)]
    pub yes: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "zsh")]
    Zsh,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum CompletionCommands {
    /// Install completions into the managed shell profile without installing presets
    Install,
    /// Generate bash completion registration script
    Bash,
    /// Generate PowerShell completion registration script
    #[command(name = "powershell")]
    PowerShell,
    /// Generate zsh completion registration script
    Zsh,
}

impl CompletionCommands {
    pub fn generate(self) {
        match self {
            CompletionCommands::Bash => completion::generate_registration(CompletionShell::Bash),
            CompletionCommands::PowerShell => {
                completion::generate_registration(CompletionShell::PowerShell)
            }
            CompletionCommands::Zsh => completion::generate_registration(CompletionShell::Zsh),
            CompletionCommands::Install => unreachable!("install is handled by the async runtime"),
        }
    }
}

impl CompletionShell {
    pub fn from_command(command: &CompletionCommands) -> Option<Self> {
        match command {
            CompletionCommands::Bash => Some(CompletionShell::Bash),
            CompletionCommands::PowerShell => Some(CompletionShell::PowerShell),
            CompletionCommands::Zsh => Some(CompletionShell::Zsh),
            CompletionCommands::Install => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CompletionShell::Bash => "bash",
            CompletionShell::PowerShell => "powershell",
            CompletionShell::Zsh => "zsh",
        }
    }
}

#[derive(Parser, Debug)]
pub struct UpdateCommand {
    /// Installed shell or app target to inspect (shows pending content differences)
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,
    /// Pull Git-managed preset sources before checking status
    #[arg(long)]
    pub pull: bool,
    /// Show content differences for available shell and app updates
    #[arg(long)]
    pub diff: bool,
    /// Show installed entries that are already current or need attention
    #[arg(long, conflicts_with = "target")]
    pub verbose: bool,
    /// Bypass the 24-hour version cache and check GitHub now
    #[arg(long, conflicts_with = "target")]
    pub refresh_release: bool,
}

#[derive(Parser, Debug)]
pub struct UpgradeCommand {
    /// Installed app, shell, or managed sys target to upgrade
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,
    /// Pull Git-managed preset sources before upgrading installed configs
    #[arg(long)]
    pub pull: bool,
    /// Show detailed env-template checks and skipped rows
    #[arg(long)]
    pub verbose: bool,
    /// Remove stale managed app files whose preset source no longer exists
    #[arg(long)]
    pub prune_stale: bool,
}
