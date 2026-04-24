use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

mod bin_links;
mod commands;
mod config;
mod presets;
mod shells;
mod update_check;

use crate::config::Config;
use commands::{AppCommands, ShellCommands};
use update_check::UpdateStatus;

/// `Shine` - Quick config for sys
#[derive(Parser, Debug)]
#[command(name = "shine")]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(long, global = true)]
    config_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Initialize quick shells
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Check for a newer version of shine
    Update,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "fish")]
    Fish,
    #[value(name = "zsh")]
    Zsh,
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "elvish")]
    Elvish,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(config_dir) = &cli.config_dir {
        if config_dir.trim().is_empty() {
            bail!("--config-dir is required when using --config-dir")
        }
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", config_dir) }
    }

    let config = Box::pin(Config::load_or_init()).await?;

    // Skip the background version check when the user explicitly runs `shine update`,
    // which does its own forced fetch below.
    if !matches!(cli.command, Commands::Update) {
        match update_check::check_for_update(&config).await {
            Ok(UpdateStatus::UpToDate) => {}
            Ok(UpdateStatus::UpdateAvailable { latest }) => {
                eprintln!(
                    "A newer version of shine is available: {} -> {}. Please update when convenient.",
                    env!("CARGO_PKG_VERSION"),
                    latest
                );
            }
            Ok(UpdateStatus::UpdateRequired { latest }) => {
                bail!(
                    "A newer patch release of shine is required: {} -> {}. Please update before continuing.",
                    env!("CARGO_PKG_VERSION"),
                    latest
                );
            }
            Err(_) => {}
        }
    }

    if let Commands::App { command: _ } = &cli.command {}

    if let Commands::Completions { shell: _ } = &cli.command {
        let _stdout = std::io::stdout().lock();
        return Ok(());
    }

    match cli.command {
        Commands::App { .. } => unreachable!(),
        Commands::Completions { .. } => unreachable!(),
        Commands::Update => handle_update(&config).await,
        Commands::Shell { command } => match command {
            ShellCommands::List => Box::pin(shells::handle_list()).await,
            ShellCommands::Install { category } => {
                Box::pin(shells::handle_install(&config, category.as_deref())).await
            }
            ShellCommands::Uninstall { purge, dry_run } => {
                Box::pin(shells::handle_uninstall(&config, purge, dry_run)).await
            }
        },
    }
}

async fn handle_update(config: &Config) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Checking for updates (current: {current})...");

    match update_check::check_for_update_forced(config).await {
        Ok(UpdateStatus::UpToDate) => {
            println!("shine {current} is up to date.");
        }
        Ok(UpdateStatus::UpdateAvailable { latest }) => {
            println!("A newer version of shine is available: {current} -> {latest}.");
            println!("Please update when convenient.");
        }
        Ok(UpdateStatus::UpdateRequired { latest }) => {
            bail!(
                "A newer patch release of shine is required: {current} -> {latest}. Please update before continuing."
            );
        }
        Err(e) => {
            bail!("Update check failed: {e}");
        }
    }

    Ok(())
}
