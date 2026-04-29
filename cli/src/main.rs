use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

mod apps;
mod bin_links;
mod check;
mod colors;
mod commands;
mod config;
mod list;
mod presets;
mod shells;
mod update_check;

use crate::config::Config;
use commands::{AppCommands, CheckCommands, ShellCommands};
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
    /// Install app config files (e.g. starship.toml, .ideavimrc) to their annotated destinations
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    /// Check which shine configurations are applied locally
    Check {
        #[command(subcommand)]
        command: Option<CheckCommands>,
    },
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// List installed shell presets and app configs
    List,
    /// Check for a newer version of shine
    Update,
    /// Download and install the latest shine release for this platform
    Upgrade,
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

    // Skip the background version check when the user explicitly runs `shine update`
    // or `shine upgrade`, which do their own forced fetch below.
    if !matches!(cli.command, Commands::Update | Commands::Upgrade) {
        match update_check::check_for_update(&config).await {
            Ok(UpdateStatus::UpToDate) => {}
            Ok(UpdateStatus::UpdateAvailable { latest }) => {
                eprintln!(
                    "A newer version of shine is available: {} -> {}. Run `shine upgrade` when convenient.",
                    env!("CARGO_PKG_VERSION"),
                    latest
                );
            }
            Ok(UpdateStatus::UpdateRequired { latest }) => {
                bail!(
                    "A newer patch release of shine is required: {} -> {}. Run `shine upgrade` before continuing.",
                    env!("CARGO_PKG_VERSION"),
                    latest
                );
            }
            Err(_) => {}
        }
    }

    if let Commands::Completions { shell: _ } = &cli.command {
        let _stdout = std::io::stdout().lock();
        return Ok(());
    }

    match cli.command {
        Commands::Completions { .. } => unreachable!(),
        Commands::App { command } => match command {
            AppCommands::List => Box::pin(apps::handle_list(&config)).await,
            AppCommands::Info { category } => Box::pin(apps::handle_info(&config, &category)).await,
            AppCommands::Install {
                category,
                force,
                dry_run,
            } => Box::pin(apps::handle_install(&config, category, dry_run, force)).await,
            AppCommands::Uninstall {
                category,
                purge,
                dry_run,
            } => {
                Box::pin(apps::handle_uninstall(
                    &config,
                    category.as_deref(),
                    purge,
                    dry_run,
                ))
                .await
            }
        },
        Commands::Update => handle_update(&config).await,
        Commands::Upgrade => handle_upgrade(&config).await,
        Commands::List => Box::pin(list::handle_list(&config)).await,
        Commands::Check { command } => Box::pin(check::handle_check(&config, command)).await,
        Commands::Shell { command } => match command {
            ShellCommands::List => Box::pin(shells::handle_list(&config)).await,
            ShellCommands::Install { category, force } => {
                Box::pin(shells::handle_install(&config, category.as_deref(), force)).await
            }
            ShellCommands::Uninstall {
                category,
                purge,
                dry_run,
            } => {
                Box::pin(shells::handle_uninstall(
                    &config,
                    category.as_deref(),
                    purge,
                    dry_run,
                ))
                .await
            }
        },
    }
}

async fn handle_update(config: &Config) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Checking for updates (current: {current})...");

    match update_check::check_for_update_forced(config).await {
        Ok(UpdateStatus::UpToDate) => {
            println!(
                "{}",
                colors::green(&format!("shine {current} is up to date."))
            );
        }
        Ok(UpdateStatus::UpdateAvailable { latest }) => {
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer version of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine upgrade` to install it.");
        }
        Ok(UpdateStatus::UpdateRequired { latest }) => {
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer patch release of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine upgrade` to install it.");
        }
        Err(e) => {
            eprintln!("Update check failed: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn handle_upgrade(config: &Config) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Checking for upgrades (current: {current})...");

    match update_check::upgrade_to_latest_release(config).await {
        Ok(update_check::UpgradeResult::AlreadyUpToDate) => {
            println!(
                "{}",
                colors::green(&format!("shine {current} is up to date."))
            );
        }
        Ok(update_check::UpgradeResult::Upgraded { previous, latest }) => {
            println!(
                "{}",
                colors::green(&format!("Upgraded shine from {previous} to {latest}."))
            );
        }
        Err(e) => {
            bail!("Upgrade failed: {e}");
        }
    }

    Ok(())
}
