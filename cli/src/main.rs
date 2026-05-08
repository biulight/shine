use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Generator, generate};
use std::path::PathBuf;

mod apps;
mod bin_links;
mod check;
mod colors;
mod commands;
mod config;
mod env;
mod list;
mod presets;
mod secret;
mod shells;
mod show;
mod sys;
mod update_check;
mod version;

use crate::config::Config;
use commands::{
    AppCommands, EnvCommands, ExportCommand, LinkCommand, SelfCommands, ShellCommands, SysCommands,
};
use update_check::{ReleaseChannel, UpdateStatus};

/// `Shine` - Quick config for sys
#[derive(Parser, Debug)]
#[command(name = "shine")]
#[command(version = crate::version::display(), about, long_about = None)]
struct Cli {
    #[arg(long, global = true)]
    config_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
    #[command(about = "Generate shell completion scripts for manual installation")]
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// List installed shell presets and app configs
    List,
    /// Show the full content and details for an installed config or shell preset
    Show {
        /// Installed item to show (e.g. git, starship, proxy, setproxy)
        #[arg(value_name = "TARGET")]
        target: String,
    },
    /// Copy built-in presets to a directory for local customization
    Export(ExportCommand),
    /// Set the external presets directory in the active config
    Link(LinkCommand),
    /// Remove the external presets directory from the active config
    Unlink,
    /// Show installed config status and check for a newer version of shine
    Update(UpdateCommand),
    /// Force-update installed shell and app configs
    Upgrade(UpgradeCommand),
    /// Manage the shine binary itself
    #[command(name = "self")]
    Self_ {
        #[command(subcommand)]
        command: SelfCommands,
    },
    /// Manage environment variables used during preset installation
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
    /// Initialize or inspect system-level presets for the current OS
    Sys {
        #[command(subcommand)]
        command: SysCommands,
    },
}

#[derive(Args, Debug)]
struct InitCommand {
    /// Skip the confirmation prompt
    #[arg(long)]
    yes: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "fish")]
    Fish,
    #[value(name = "zsh")]
    Zsh,
}

impl CompletionShell {
    fn generate(self) {
        let mut command = Cli::command();
        let mut stdout = std::io::stdout();
        match self {
            CompletionShell::Bash => {
                write_completions(clap_complete::shells::Bash, &mut command, &mut stdout)
            }
            CompletionShell::Fish => {
                write_completions(clap_complete::shells::Fish, &mut command, &mut stdout)
            }
            CompletionShell::Zsh => {
                write_completions(clap_complete::shells::Zsh, &mut command, &mut stdout)
            }
        }
    }
}

#[derive(Parser, Debug)]
struct UpdateCommand {
    /// Show installed entries that are already current or need attention
    #[arg(long)]
    verbose: bool,
}

#[derive(Parser, Debug)]
struct UpgradeCommand {
    /// Show detailed env-template checks and skipped rows
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::Completions { shell } = &cli.command {
        shell.generate();
        return Ok(());
    }

    if let Some(config_dir) = &cli.config_dir {
        if config_dir.trim().is_empty() {
            bail!("--config-dir is required when using --config-dir")
        }
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", config_dir) }
    }

    if let Commands::Init(cmd) = &cli.command {
        return handle_init(cmd.yes).await;
    }

    let config = Box::pin(Config::load_or_init()).await?;

    // Skip the background version check for update/self commands. `shine update`
    // and `shine self upgrade` do their own forced fetch below; `shine self install`
    // should remain available even when the current binary is version-gated.
    if !matches!(
        cli.command,
        Commands::Update(..)
            | Commands::Export(..)
            | Commands::Link(..)
            | Commands::Unlink
            | Commands::Self_ { .. }
            | Commands::Env { .. }
    ) {
        match update_check::check_for_update(&config).await {
            Ok(UpdateStatus::UpToDate) => {}
            Ok(UpdateStatus::UpdateAvailable { latest }) => {
                eprintln!(
                    "A newer version of shine is available: {} -> {}. Run `shine self upgrade` when convenient.",
                    version::display(),
                    latest
                );
            }
            Ok(UpdateStatus::UpdateRequired { latest }) => {
                bail!(
                    "A newer patch release of shine is required: {} -> {}. Run `shine self upgrade` before continuing.",
                    version::display(),
                    latest
                );
            }
            Err(_) => {}
        }
    }

    match cli.command {
        Commands::Init(_) => unreachable!(),
        Commands::Completions { .. } => unreachable!(),
        Commands::App { command } => match command {
            AppCommands::Init { force } => apps::handle_init_template(force).await,
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
        Commands::Update(cmd) => handle_update(&config, cmd.verbose).await,
        Commands::Upgrade(cmd) => handle_config_upgrade(&config, cmd.verbose).await,
        Commands::Export(ExportCommand { dir, force }) => {
            Box::pin(handle_presets_export(&config, dir, force)).await
        }
        Commands::Link(LinkCommand { path, create }) => {
            Box::pin(handle_presets_link(&config, path, create)).await
        }
        Commands::Unlink => Box::pin(handle_presets_unlink(&config)).await,
        Commands::List => Box::pin(list::handle_list(&config)).await,
        Commands::Show { target } => Box::pin(show::handle_show(&config, &target)).await,
        Commands::Self_ { command } => match command {
            SelfCommands::Install { dest } => handle_self_install(config.clone(), dest).await,
            SelfCommands::Upgrade { channel } => handle_self_upgrade(&config, channel).await,
        },
        Commands::Shell { command } => match command {
            ShellCommands::Init { force } => shells::handle_init_template(force).await,
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
        Commands::Env { command } => match command {
            EnvCommands::Show => handle_env_show(&config).await,
            EnvCommands::Set { key, value } => handle_env_set(&config, key, value).await,
            EnvCommands::Get { key } => handle_env_get(&config, key).await,
            EnvCommands::Decrypt { key } => handle_env_decrypt(&config, key).await,
        },
        Commands::Sys { command } => match command {
            SysCommands::List => Box::pin(sys::handle_list(&config)).await,
            SysCommands::Init { dry_run } => Box::pin(sys::handle_init(&config, dry_run)).await,
        },
    }
}

async fn handle_init(yes: bool) -> Result<()> {
    let current_dir = std::env::current_dir()?;
    let display_dir = tokio::fs::canonicalize(&current_dir)
        .await
        .unwrap_or(current_dir);

    if !yes && !confirm_init(&display_dir)? {
        println!("{}", colors::dim("Init cancelled."));
        return Ok(());
    }

    let path = Config::init_current_dir_config().await?;
    println!(
        "{}",
        colors::green(&format!("Initialized shine config at {}", path.display()))
    );
    println!(
        "{}",
        colors::dim(&format!("presets_dir = {}", display_dir.display()))
    );
    Ok(())
}

fn confirm_init(dir: &std::path::Path) -> Result<bool> {
    use std::io::Write as _;

    print!(
        "Initialize {} as the shine presets directory? [y/N] ",
        dir.display()
    );
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let answer = input.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn write_completions<G: Generator>(
    generator: G,
    command: &mut clap::Command,
    out: &mut dyn std::io::Write,
) {
    generate(generator, command, command.get_name().to_string(), out);
}

async fn handle_env_show(config: &Config) -> Result<()> {
    let env = env::EnvConfig::load_or_init(config).await?;
    println!(
        "{}",
        colors::dim(&format!("# {} [env]", config.config_path().display()))
    );
    for (k, v) in env.iter() {
        println!("{k} = \"{v}\"");
    }
    Ok(())
}

async fn handle_env_set(config: &Config, key: String, value: String) -> Result<()> {
    let mut env = env::EnvConfig::load_or_init(config).await?;
    env.set(&key, &value);
    env.save(config).await?;
    println!("{}", colors::green(&format!("set {key} = \"{value}\"")));
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

async fn handle_env_get(config: &Config, key: String) -> Result<()> {
    let env = env::EnvConfig::load_or_init(config).await?;
    match env.get(&key) {
        Some(v) => println!("{v}"),
        None => {
            eprintln!(
                "{}",
                colors::yellow(&format!("{key} is not set in the active config [env]"))
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn handle_env_decrypt(config: &Config, key: String) -> Result<()> {
    let env = env::EnvConfig::load_or_init(config).await?;
    let Some(value) = env.get(&key) else {
        bail!("{key} is not set in the active config [env]");
    };
    let plaintext = secret::decrypt_base64_gpg_secret(value)
        .await
        .with_context(|| format!("decrypting {key}"))?;
    print!("{plaintext}");
    Ok(())
}

async fn handle_update(config: &Config, verbose: bool) -> Result<()> {
    let mut printed_update = if verbose {
        Box::pin(list::handle_status_list(config)).await?;
        println!();
        true
    } else {
        Box::pin(list::handle_update_list(config)).await?
    };

    let current = version::display();
    if verbose {
        println!("Checking for updates (current: {current})...");
    }

    match update_check::check_for_update_forced(config).await {
        Ok(UpdateStatus::UpToDate) => {
            if verbose {
                println!(
                    "{}",
                    colors::green(&format!("shine {current} is up to date."))
                );
            }
        }
        Ok(UpdateStatus::UpdateAvailable { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer version of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Ok(UpdateStatus::UpdateRequired { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer patch release of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Err(e) => {
            eprintln!("Update check failed: {e}");
            std::process::exit(1);
        }
    }

    if !printed_update {
        println!("{}", colors::dim("Nothing to update."));
    }

    Ok(())
}

async fn handle_self_upgrade(config: &Config, channel: Option<ReleaseChannel>) -> Result<()> {
    let current = version::display();
    let selected_channel = channel.unwrap_or(ReleaseChannel::Stable);
    let force_install = channel.is_some();
    println!(
        "Checking for {} upgrades (current: {current})...",
        selected_channel.as_str()
    );

    match update_check::upgrade_to_release(config, selected_channel, force_install).await {
        Ok(update_check::UpgradeResult::AlreadyUpToDate { channel, latest }) => {
            println!(
                "{}",
                colors::green(&format!(
                    "shine {current} is up to date on the {} channel ({latest}).",
                    channel.as_str()
                ))
            );
        }
        Ok(update_check::UpgradeResult::Upgraded {
            channel,
            previous: _,
            release_tag,
            installed_path,
        }) => {
            match channel {
                ReleaseChannel::Stable => println!(
                    "{}",
                    colors::green(&format!("Upgraded shine from {current} to {release_tag}."))
                ),
                ReleaseChannel::Preview => println!(
                    "{}",
                    colors::green(&format!(
                        "Installed shine preview from {release_tag} over {current}."
                    ))
                ),
            }
            sync_self_install_dest(config, &installed_path).await;
        }
        Err(e) => {
            bail!("Upgrade failed: {e}");
        }
    }

    Ok(())
}

async fn handle_config_upgrade(config: &Config, verbose: bool) -> Result<()> {
    println!("{}", colors::bold("Upgrading installed configs"));
    crate::config::print_presets_note(config);

    let env_report = Box::pin(env::upgrade::handle_upgrade(config, false, verbose)).await?;
    let shell_report = Box::pin(shells::handle_upgrade_installed(config, verbose)).await?;
    let app_report = Box::pin(apps::handle_upgrade_installed(config)).await?;

    let updated = env_report.updated
        + shell_report.templates_updated
        + shell_report.links_created
        + shell_report.links_updated
        + usize::from(shell_report.path_changed)
        + app_report.updated;
    let skipped = env_report.skipped + app_report.skipped;
    let user_modified = env_report.user_modified + app_report.user_modified;

    let mut summary: Vec<String> = Vec::new();
    if updated > 0 {
        summary.push(colors::green(&format!("{updated} updated")));
    }
    if user_modified > 0 {
        summary.push(colors::yellow(&format!(
            "{user_modified} user-modified (kept)"
        )));
    }
    if shell_report.link_conflicts > 0 {
        summary.push(colors::yellow(&format!(
            "{} link conflicts",
            shell_report.link_conflicts
        )));
    }
    if skipped > 0 {
        summary.push(colors::dim(&format!("{skipped} skipped")));
    }
    if summary.is_empty() {
        summary.push(colors::dim("nothing changed"));
    }
    let sep = colors::dim(" · ");
    println!("\n{}  {}", colors::bold("Done"), summary.join(&sep));

    Ok(())
}

/// After a successful self-upgrade, try to sync the new binary to the self-install destination.
/// If the copy fails due to permissions, print a targeted hint instead of failing.
async fn sync_self_install_dest(config: &Config, src: &std::path::Path) {
    let dest = match &config.self_install_dest {
        Some(d) => d,
        None => return,
    };
    match sync_self_install_dest_from(src, dest) {
        Ok(SelfInstallSync::Synced) => println!(
            "{}",
            colors::green(&format!("Synced system copy at {}", dest.display()))
        ),
        Ok(SelfInstallSync::AlreadyCurrent) => {}
        Err(e) if has_io_error_kind(&e, std::io::ErrorKind::PermissionDenied) => println!(
            "{}",
            colors::yellow(&format!(
                "System copy at {} needs manual sync — run: sudo {} self install",
                dest.display(),
                src.display()
            ))
        ),
        Err(e) => eprintln!(
            "Warning: failed to sync system copy at {}: {e}",
            dest.display()
        ),
    }
}

enum SelfInstallSync {
    Synced,
    AlreadyCurrent,
}

fn sync_self_install_dest_from(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> Result<SelfInstallSync> {
    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
        if canonical_src == canonical_dest {
            return Ok(SelfInstallSync::AlreadyCurrent);
        }
    }

    install_binary_atomically(src, dest).map(|()| SelfInstallSync::Synced)
}

fn has_io_error_kind(err: &anyhow::Error, kind: std::io::ErrorKind) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == kind)
    })
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
            "Tip: run `shine link {}` to activate this directory.",
            target.display()
        );
    }

    Ok(())
}

async fn handle_presets_link(config: &Config, path: PathBuf, create: bool) -> Result<()> {
    use anyhow::Context as _;

    let raw = path.to_string_lossy();
    let expanded =
        crate::config::full_expand(&raw).with_context(|| format!("expanding path: {raw}"))?;
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
                 the active config at runtime. Unset the env var for this setting to take effect."
            )
        );
    }

    println!("{}", colors::external_presets_note(&absolute));
    println!(
        "{}",
        colors::dim("Run `shine export` to populate the directory with built-in presets.")
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
        colors::green("External presets directory removed from the active config.")
    );
    println!(
        "{}",
        colors::dim("Built-in embedded presets will be used on the next run.")
    );

    Ok(())
}

async fn handle_self_install(mut config: Config, dest: std::path::PathBuf) -> Result<()> {
    use anyhow::{Context as _, bail};

    let src = std::env::current_exe().context("failed to resolve current executable path")?;

    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.clone());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.clone());
        if canonical_src == canonical_dest {
            bail!(
                "source and destination are the same binary: {}. Run the newer binary by full path, e.g. `sudo /path/to/new/shine self install`, to overwrite this copy.",
                dest.display()
            );
        }
    }

    install_binary_atomically(&src, &dest).with_context(|| {
        format!(
            "failed to copy to {} — try: sudo {} self install",
            dest.display(),
            src.display()
        )
    })?;

    // Remember where we installed so `shine self upgrade` can sync this copy automatically.
    config.self_install_dest = Some(dest.clone());
    config
        .save()
        .await
        .context("failed to save self_install_dest to config")?;

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

fn install_binary_atomically(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    use anyhow::Context as _;

    let parent = dest
        .parent()
        .with_context(|| format!("destination has no parent: {}", dest.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create destination dir: {}", parent.display()))?;

    let temp = parent.join(format!(".shine-self-install-{}", uuid::Uuid::new_v4()));
    std::fs::copy(src, &temp).with_context(|| {
        format!(
            "failed to stage binary from {} to {}",
            src.display(),
            temp.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(src)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o755);
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode));
    }

    match std::fs::rename(&temp, dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            Err(err)
                .with_context(|| format!("failed to replace {} with staged binary", dest.display()))
        }
    }
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

    #[test]
    fn install_binary_atomically_overwrites_existing_dest() {
        let dir = std::env::temp_dir().join(format!("shine-self-install-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("new-shine");
        let dest = dir.join("shine");

        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dest, b"old").unwrap();

        install_binary_atomically(&src, &dest).unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sync_self_install_dest_creates_missing_parent() {
        let dir = std::env::temp_dir().join(format!("shine-self-sync-{}", uuid::Uuid::new_v4()));
        let src = dir.join("new-shine");
        let dest = dir.join("usr/local/bin/shine");

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src, b"new").unwrap();

        let outcome = sync_self_install_dest_from(&src, &dest).unwrap();

        assert!(matches!(outcome, SelfInstallSync::Synced));
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sync_self_install_dest_skips_current_exe_path() {
        let dir = std::env::temp_dir().join(format!("shine-self-sync-{}", uuid::Uuid::new_v4()));
        let src = dir.join("shine");

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src, b"new").unwrap();

        let outcome = sync_self_install_dest_from(&src, &src).unwrap();

        assert!(matches!(outcome, SelfInstallSync::AlreadyCurrent));
        assert_eq!(std::fs::read(&src).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn self_install_errors_when_source_is_destination() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let current = std::env::current_exe().unwrap();

        let err = handle_self_install(config, current).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("source and destination are the same binary"),
            "error should explain self-overwrite: {err:#}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn cli_accepts_refactored_update_commands() {
        let cli = Cli::try_parse_from(["shine", "self", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Self_ {
                command: SelfCommands::Upgrade { channel: None }
            }
        ));

        let cli =
            Cli::try_parse_from(["shine", "self", "upgrade", "--channel", "preview"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Self_ {
                command: SelfCommands::Upgrade {
                    channel: Some(ReleaseChannel::Preview)
                }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "update"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand { verbose: false })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand { verbose: true })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand { verbose: false })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand { verbose: true })
        ));
    }

    #[test]
    fn cli_accepts_top_level_presets_commands() {
        let cli = Cli::try_parse_from(["shine", "export"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Export(ExportCommand {
                dir: None,
                force: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "link", "/tmp/presets", "--create"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Link(LinkCommand { create: true, .. })
        ));

        let cli = Cli::try_parse_from(["shine", "unlink"]).unwrap();
        assert!(matches!(cli.command, Commands::Unlink));
    }

    #[test]
    fn cli_accepts_top_level_init_command() {
        let cli = Cli::try_parse_from(["shine", "init"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Init(InitCommand { yes: false })
        ));

        let cli = Cli::try_parse_from(["shine", "init", "--yes"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Init(InitCommand { yes: true })
        ));
    }

    #[test]
    fn cli_accepts_show_command() {
        let cli = Cli::try_parse_from(["shine", "show", "setproxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Show { target } if target == "setproxy"
        ));
    }

    #[test]
    fn cli_accepts_shell_and_app_init_commands() {
        let cli = Cli::try_parse_from(["shine", "shell", "init"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Init { force: false }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "shell", "init", "--force"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Init { force: true }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "app", "init"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Init { force: false }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "app", "init", "-f"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Init { force: true }
            }
        ));
    }

    #[test]
    fn cli_completions_rejects_unsupported_shells() {
        assert!(Cli::try_parse_from(["shine", "completions", "powershell"]).is_err());
        assert!(Cli::try_parse_from(["shine", "completions", "elvish"]).is_err());
    }

    #[test]
    fn cli_completions_accepts_supported_shells() {
        assert!(Cli::try_parse_from(["shine", "completions", "bash"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "completions", "fish"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "completions", "zsh"]).is_ok());
    }

    #[test]
    fn completions_output_is_non_empty_for_supported_shells() {
        for shell in [
            CompletionShell::Bash,
            CompletionShell::Fish,
            CompletionShell::Zsh,
        ] {
            let mut command = Cli::command();
            let mut output = Vec::new();

            match shell {
                CompletionShell::Bash => {
                    write_completions(clap_complete::shells::Bash, &mut command, &mut output)
                }
                CompletionShell::Fish => {
                    write_completions(clap_complete::shells::Fish, &mut command, &mut output)
                }
                CompletionShell::Zsh => {
                    write_completions(clap_complete::shells::Zsh, &mut command, &mut output)
                }
            }

            let script = String::from_utf8(output).unwrap();
            assert!(
                !script.trim().is_empty(),
                "completion script should not be empty"
            );
            assert!(
                script.contains("shine"),
                "completion script should mention the command name"
            );
        }
    }

    #[test]
    fn cli_rejects_removed_env_upgrade_commands() {
        assert!(Cli::try_parse_from(["shine", "env", "upgrade"]).is_err());
        assert!(Cli::try_parse_from(["shine", "env", "update"]).is_err());
        assert!(Cli::try_parse_from(["shine", "env", "path"]).is_err());
    }

    #[test]
    fn cli_rejects_removed_check_commands() {
        assert!(Cli::try_parse_from(["shine", "check"]).is_err());
        assert!(Cli::try_parse_from(["shine", "check", "app"]).is_err());
        assert!(Cli::try_parse_from(["shine", "check", "shell"]).is_err());
    }

    #[test]
    fn cli_rejects_removed_presets_subcommands() {
        assert!(Cli::try_parse_from(["shine", "presets"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "export"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "link", "/tmp/presets"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "unlink"]).is_err());
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
