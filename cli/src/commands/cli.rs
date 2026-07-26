use crate::completion;
use crate::version;
use clap::{Args, Parser, Subcommand, ValueEnum};

use super::{
    AppCommands, EnvCommands, ExportCommand, LinkCommand, LocalCommands, OverlayCommands,
    SelfCommands, ServeCommands, ShellCommands, SysCommands, TaskCommands, TaskRunCommand,
    ThemeCommands,
};

/// `Shine` - Quick config for sys
#[derive(Parser, Debug)]
#[command(name = "shine")]
#[command(version = version::display(), about, long_about = None)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize the current directory as a shine presets directory
    Init(InitCommand),
    /// Initialize quick shells
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },
    /// Install app config files (e.g. starship.toml, .ideavimrc) to their annotated destinations
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    /// Install a shell or app preset category
    Install {
        /// Preset category to install (e.g. proxy, starship)
        #[arg(value_name = "CATEGORY")]
        category: String,
    },
    /// Reinstall a shell or app preset category
    Reinstall {
        /// Preset category to reinstall (e.g. proxy, starship)
        #[arg(value_name = "CATEGORY")]
        category: String,
    },
    /// Uninstall a shell or app preset category
    Uninstall {
        /// Preset category to uninstall (e.g. proxy, starship)
        #[arg(value_name = "CATEGORY")]
        category: String,
    },
    /// Generate or install shell completion scripts
    Completions {
        #[command(subcommand)]
        command: CompletionCommands,
    },
    /// List installed shell presets, app configs, and managed system configs
    List,
    /// Show details for an installed config or shell preset
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
    /// Copy built-in presets to a directory for local customization
    Export(ExportCommand),
    /// Set the external presets directory in the active config
    Link(LinkCommand),
    /// Remove the external presets directory from the active config
    Unlink,
    /// Manage the personal presets overlay directory
    Overlay {
        #[command(subcommand)]
        command: OverlayCommands,
    },
    /// Pull Git-managed preset and overlay repositories
    Pull,
    /// Show installed config status and check for a newer version of shine
    Update(UpdateCommand),
    /// Force-update installed shell and app configs
    Upgrade(UpgradeCommand),
    /// Clear old shine-owned runtime state after schema changes
    Clear(ClearCommand),
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
    /// Initialize or inspect system-level presets for the current OS
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
    /// Pull Git-managed preset sources before checking status
    #[arg(long)]
    pub pull: bool,
    /// Show installed entries that are already current or need attention
    #[arg(long)]
    pub verbose: bool,
    /// Bypass the 24-hour version cache and check GitHub now
    #[arg(long)]
    pub refresh: bool,
}

#[derive(Parser, Debug)]
pub struct UpgradeCommand {
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

#[derive(Parser, Debug)]
pub struct ClearCommand {
    /// Print cleanup steps without changing files
    #[arg(long)]
    pub dry_run: bool,
}
