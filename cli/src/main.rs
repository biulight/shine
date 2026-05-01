use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

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
use commands::{AppCommands, CheckCommands, PresetsCommands, SelfCommands, ShellCommands};
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

#[derive(Subcommand, Debug)]
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
    /// Manage the external presets directory (link, unlink, export)
    Presets {
        #[command(subcommand)]
        command: PresetsCommands,
    },
    /// Check for a newer version of shine
    Update,
    /// Download and install the latest shine release for this platform
    Upgrade,
    /// Manage the shine binary itself
    #[command(name = "self")]
    Self_ {
        #[command(subcommand)]
        command: SelfCommands,
    },
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
    if !matches!(
        cli.command,
        Commands::Update | Commands::Upgrade | Commands::Presets { .. } | Commands::Self_ { .. }
    ) {
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
        Commands::Presets { command } => match command {
            PresetsCommands::Export { dir, force } => {
                Box::pin(handle_presets_export(&config, dir, force)).await
            }
            PresetsCommands::Link { path, create } => {
                Box::pin(handle_presets_link(&config, path, create)).await
            }
            PresetsCommands::Unlink => Box::pin(handle_presets_unlink(&config)).await,
        },
        Commands::List => Box::pin(list::handle_list(&config)).await,
        Commands::Check { command } => Box::pin(check::handle_check(&config, command)).await,
        Commands::Self_ { command } => match command {
            SelfCommands::Install { dest } => handle_self_install(dest).await,
        },
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

async fn handle_presets_export(config: &Config, dir: Option<PathBuf>, force: bool) -> Result<()> {
    use anyhow::Context as _;

    let target = dir.unwrap_or_else(|| config.presets_dir().to_owned());
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("creating export directory: {}", target.display()))?;

    println!("Exporting built-in presets to {} ...", target.display());

    let report = presets::extract_all(&target, force).await?;

    let created = report.created.len();
    let overwritten = report.overwritten.len();
    let skipped = report.skipped.len();

    if created > 0 {
        println!("{}", colors::green(&format!("  {created} file(s) created")));
    }
    if overwritten > 0 {
        println!(
            "{}",
            colors::yellow(&format!("  {overwritten} file(s) updated (overwritten)"))
        );
    }
    if skipped > 0 {
        println!("  {skipped} file(s) skipped (already exist; use --force to overwrite)");
    }
    if created == 0 && overwritten == 0 && skipped == 0 {
        println!("  No files exported (empty embedded asset set).");
    }

    if !config.is_external_presets {
        println!();
        println!(
            "Tip: run `shine presets link {}` to activate this directory.",
            target.display()
        );
    }

    Ok(())
}

async fn handle_presets_link(config: &Config, path: PathBuf, create: bool) -> Result<()> {
    use anyhow::Context as _;

    let raw = path.to_string_lossy();
    let expanded = shellexpand::full(&raw)
        .with_context(|| format!("expanding path: {raw}"))?
        .to_string();
    let expanded = PathBuf::from(expanded);

    if create {
        tokio::fs::create_dir_all(&expanded)
            .await
            .with_context(|| format!("creating directory: {}", expanded.display()))?;
    }

    let meta = tokio::fs::metadata(&expanded).await.with_context(|| {
        if create {
            format!("accessing directory: {}", expanded.display())
        } else {
            format!(
                "path does not exist: {} (use --create to create it)",
                expanded.display()
            )
        }
    })?;

    if !meta.is_dir() {
        bail!("path is not a directory: {}", expanded.display());
    }

    let absolute = tokio::fs::canonicalize(&expanded).await.unwrap_or(expanded);

    if config
        .presets_dir_override
        .as_deref()
        .is_some_and(|p| p == absolute)
    {
        println!(
            "{}",
            colors::dim(&format!("already linked: {}", absolute.display()))
        );
        return Ok(());
    }

    let updated = config
        .clone()
        .with_presets_dir_override(Some(absolute.clone()));
    updated.save().await?;

    if std::env::var("SHINE_CONFIG_DIR")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
        || std::env::var("SHINE_PRESETS")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    {
        println!(
            "{}",
            colors::yellow(
                "Warning: SHINE_CONFIG_DIR or SHINE_PRESETS is set and takes priority over \
                 config.toml at runtime. Unset the env var for this setting to take effect."
            )
        );
    }

    println!("{}", colors::external_presets_note(&absolute));
    println!(
        "{}",
        colors::dim("Run `shine presets export` to populate the directory with built-in presets.")
    );

    Ok(())
}

async fn handle_presets_unlink(config: &Config) -> Result<()> {
    if config.presets_dir_override.is_none() {
        println!(
            "{}",
            colors::dim("No external presets directory is configured.")
        );
        return Ok(());
    }

    let updated = config.clone().with_presets_dir_override(None);
    updated.save().await?;

    println!(
        "{}",
        colors::green("External presets directory removed from config.toml.")
    );
    println!(
        "{}",
        colors::dim("Built-in embedded presets will be used on the next run.")
    );

    Ok(())
}

async fn handle_self_install(dest: std::path::PathBuf) -> Result<()> {
    use anyhow::Context as _;

    let src = std::env::current_exe().context("failed to resolve current executable path")?;

    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.clone());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.clone());
        if canonical_src == canonical_dest {
            println!(
                "{}",
                colors::dim(&format!("already installed at {}", dest.display()))
            );
            return Ok(());
        }
    }

    std::fs::copy(&src, &dest).with_context(|| {
        format!(
            "failed to copy to {} — try: sudo shine self install",
            dest.display()
        )
    })?;

    println!(
        "{}",
        colors::green(&format!("installed to {}", dest.display()))
    );
    println!(
        "{}",
        colors::dim("You can now run `sudo shine` without specifying the full path.")
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use tokio::fs;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock must not be poisoned")
    }

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-main-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn config_in(dir: &std::path::Path) -> Config {
        Config::new_for_test(dir)
    }

    #[tokio::test]
    async fn link_writes_presets_dir_to_config() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir);

        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();

        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        assert!(
            content.contains(presets.to_str().unwrap()),
            "config.toml should contain the linked path"
        );

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn link_creates_dir_when_create_flag_set() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let new_dir = dir.join("new-presets");

        handle_presets_link(&config, new_dir.clone(), true)
            .await
            .unwrap();

        assert!(new_dir.exists(), "directory should have been created");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_fails_when_path_missing_and_no_create() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let missing = dir.join("does-not-exist");

        let err = handle_presets_link(&config, missing, false).await;
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("--create") || msg.contains("does not exist"),
            "error should mention --create: {msg}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_fails_when_path_is_a_file() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let file = dir.join("not-a-dir.txt");
        fs::write(&file, b"hello").await.unwrap();

        let err = handle_presets_link(&config, file, false).await;
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("not a directory"),
            "error should mention 'not a directory'"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_is_noop_when_already_linked_to_same_path() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let abs = tokio::fs::canonicalize(&presets)
            .await
            .unwrap_or(presets.clone());
        let config = config_in(&dir).with_presets_dir_override(Some(abs.clone()));

        // Should return Ok without error
        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();

        // Config file should not be written (config_in has no pre-existing file)
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn link_warns_when_env_var_overrides() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir);

        unsafe { std::env::set_var("SHINE_PRESETS", "/some/override") };
        // Should succeed even with env var set
        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_removes_presets_dir_key() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir).with_presets_dir_override(Some(presets.clone()));
        // Write initial config with presets_dir set
        config.save().await.unwrap();

        handle_presets_unlink(&config).await.unwrap();

        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            !parsed.contains_key("presets_dir"),
            "presets_dir key must be absent after unlink"
        );

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_is_noop_when_no_override_set() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        // Should return Ok, no file written
        handle_presets_unlink(&config).await.unwrap();
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
