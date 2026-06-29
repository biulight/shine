use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dialoguer::Select;
use std::path::PathBuf;

mod apps;
mod bin_links;
mod check;
mod clear;
mod colors;
mod commands;
mod completion;
mod config;
mod env;
mod list;
mod output;
mod path_display;
mod platform;
mod presets;
mod secret;
mod shells;
mod show;
mod sys;
#[cfg(test)]
mod test_support;
mod update_check;
mod version;

use crate::config::Config;
use commands::{
    AppCommands, EnvCommands, ExportCommand, LinkCommand, OverlayCommands, SelfCommands,
    ShellCommands, SysCommands,
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
    /// List installed shell presets and app configs
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
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "zsh")]
    Zsh,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Subcommand)]
enum CompletionCommands {
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
    fn generate(self) {
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
    fn from_command(command: &CompletionCommands) -> Option<Self> {
        match command {
            CompletionCommands::Bash => Some(CompletionShell::Bash),
            CompletionCommands::PowerShell => Some(CompletionShell::PowerShell),
            CompletionCommands::Zsh => Some(CompletionShell::Zsh),
            CompletionCommands::Install => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            CompletionShell::Bash => "bash",
            CompletionShell::PowerShell => "powershell",
            CompletionShell::Zsh => "zsh",
        }
    }
}

#[derive(Parser, Debug)]
struct UpdateCommand {
    /// Show installed entries that are already current or need attention
    #[arg(long)]
    verbose: bool,
    /// Bypass the 24-hour version cache and check GitHub now
    #[arg(long)]
    refresh: bool,
}

#[derive(Parser, Debug)]
struct UpgradeCommand {
    /// Show detailed env-template checks and skipped rows
    #[arg(long)]
    verbose: bool,
    /// Remove stale managed app files whose preset source no longer exists
    #[arg(long)]
    prune_stale: bool,
}

#[derive(Parser, Debug)]
struct ClearCommand {
    /// Print cleanup steps without changing files
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    completion::complete_from_env();

    let cli = Cli::parse();

    if let Commands::Completions { command } = &cli.command
        && CompletionShell::from_command(command).is_some()
    {
        command.generate();
        return Ok(());
    }

    if let Some(config_dir) = &cli.config_dir {
        if config_dir.trim().is_empty() {
            bail!("--config-dir is required when using --config-dir")
        }
        // SAFETY: called before the Tokio runtime starts; no other threads exist at
        // this point, so the write cannot race concurrent env reads.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", config_dir) }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build async runtime")?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    if let Commands::Init(cmd) = &cli.command {
        return handle_init(cmd.yes).await;
    }

    if let Commands::Clear(cmd) = &cli.command {
        let config = if cmd.dry_run {
            Box::pin(Config::load_global_runtime_for_dry_run()).await?
        } else {
            Box::pin(Config::load_global_runtime_or_init()).await?
        };
        return Box::pin(clear::handle_clear(&config, cmd.dry_run)).await;
    }

    let config = Box::pin(Config::load_or_init()).await?;

    warn_if_runtime_schema_pending(&cli.command).await;

    // Skip the background version check for update/self commands. `shine update`
    // and `shine self upgrade` do their own forced fetch below; `shine self install`
    // should remain available even when the current binary is version-gated.
    if !matches!(
        cli.command,
        Commands::Update(..)
            | Commands::Export(..)
            | Commands::Link(..)
            | Commands::Unlink
            | Commands::Overlay { .. }
            | Commands::Clear(..)
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
        Commands::Completions {
            command: CompletionCommands::Install,
        } => Box::pin(shells::handle_completion_install(&config)).await,
        Commands::Completions { .. } => unreachable!(),
        Commands::Clear(_) => unreachable!(),
        Commands::Install { category } => handle_install_shim(&config, &category).await,
        Commands::Reinstall { category } => handle_reinstall_shim(&config, &category).await,
        Commands::Uninstall { category } => handle_uninstall_shim(&config, &category).await,
        Commands::App { command } => match command {
            AppCommands::Init { force } => apps::handle_init_template(force).await,
            AppCommands::List => Box::pin(apps::handle_list(&config)).await,
            AppCommands::Info { category } => Box::pin(apps::handle_info(&config, &category)).await,
            AppCommands::Install { category, dry_run } => {
                Box::pin(apps::handle_install(
                    &config,
                    category.as_deref(),
                    dry_run,
                    false,
                ))
                .await
            }
            AppCommands::Reinstall { category, dry_run } => {
                Box::pin(apps::handle_install(
                    &config,
                    category.as_deref(),
                    dry_run,
                    true,
                ))
                .await
            }
            AppCommands::Uninstall {
                category,
                force,
                purge,
                dry_run,
            } => {
                Box::pin(apps::handle_uninstall(
                    &config,
                    category.as_deref(),
                    force,
                    purge,
                    dry_run,
                ))
                .await
            }
        },
        Commands::Update(cmd) => handle_update(&config, cmd.verbose, cmd.refresh).await,
        Commands::Upgrade(cmd) => {
            handle_config_upgrade(&config, cmd.verbose, cmd.prune_stale).await
        }
        Commands::Export(ExportCommand { dir, force }) => {
            Box::pin(handle_presets_export(&config, dir, force)).await
        }
        Commands::Link(LinkCommand { path, create }) => {
            Box::pin(handle_presets_link(&config, path, create)).await
        }
        Commands::Unlink => Box::pin(handle_presets_unlink(&config)).await,
        Commands::Overlay { command } => match command {
            OverlayCommands::Link(LinkCommand { path, create }) => {
                Box::pin(handle_overlay_link(&config, path, create)).await
            }
            OverlayCommands::Unlink => Box::pin(handle_overlay_unlink(&config)).await,
            OverlayCommands::Show => handle_overlay_show(&config),
        },
        Commands::List => Box::pin(list::handle_list(&config)).await,
        Commands::Info {
            target,
            diff,
            verbose,
        } => Box::pin(show::handle_show(&config, &target, diff, verbose)).await,
        Commands::Self_ { command } => match command {
            SelfCommands::Install { dest } => handle_self_install(config.clone(), dest).await,
            SelfCommands::Upgrade { channel } => handle_self_upgrade(&config, channel).await,
        },
        Commands::Shell { command } => match command {
            ShellCommands::Init { force } => shells::handle_init_template(force).await,
            ShellCommands::List => Box::pin(shells::handle_list(&config)).await,
            ShellCommands::Install { category } => {
                Box::pin(shells::handle_install(&config, category.as_deref(), false)).await
            }
            ShellCommands::Reinstall { category } => {
                Box::pin(shells::handle_install(&config, category.as_deref(), true)).await
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
            EnvCommands::Set { key, value } => handle_env_set(&config, &key, &value).await,
            EnvCommands::Delete { key } => handle_env_delete(&config, &key).await,
            EnvCommands::Get { key } => handle_env_get(&config, &key).await,
            EnvCommands::Decrypt { key } => handle_env_decrypt(&config, &key).await,
            EnvCommands::Export { key, alias } => {
                handle_env_export(&config, &key, alias.as_deref()).await
            }
            EnvCommands::Encrypt(cmd) => {
                handle_env_encrypt(
                    &config,
                    cmd.recipient.as_deref(),
                    cmd.set.as_deref(),
                    cmd.from.as_deref(),
                )
                .await
            }
        },
        Commands::Sys { command } => match command {
            SysCommands::List => Box::pin(sys::handle_list(&config)).await,
            SysCommands::Status => Box::pin(sys::handle_status(&config)).await,
            SysCommands::Init {
                preset,
                dry_run,
                force_profile,
            } => {
                Box::pin(sys::handle_init(
                    &config,
                    preset.as_deref(),
                    dry_run,
                    force_profile,
                ))
                .await
            }
        },
    }
}

async fn warn_if_runtime_schema_pending(command: &Commands) {
    if !should_warn_runtime_schema(command) {
        return;
    }

    if let Ok(schema_version) = Config::read_global_runtime_schema_version().await
        && let Some(warning) = clear::pending_schema_warning(schema_version)
    {
        eprintln!("{}", colors::yellow(&warning));
    }
}

fn should_warn_runtime_schema(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Init(_) | Commands::Completions { .. } | Commands::Clear(_)
    )
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PresetKind {
    Shell,
    App,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ShimResolution {
    Found(PresetKind),
    Conflict,
    Missing,
}

async fn handle_install_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_install(config, Some(category), false)).await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_install(config, Some(category), false, false)).await
        }
        ShimResolution::Conflict => match select_shim_kind("Install", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_install(config, Some(category), false)).await
            }
            PresetKind::App => {
                Box::pin(apps::handle_install(config, Some(category), false, false)).await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

async fn handle_reinstall_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_install(config, Some(category), true)).await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_install(config, Some(category), false, true)).await
        }
        ShimResolution::Conflict => match select_shim_kind("Reinstall", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_install(config, Some(category), true)).await
            }
            PresetKind::App => {
                Box::pin(apps::handle_install(config, Some(category), false, true)).await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

async fn handle_uninstall_shim(config: &Config, category: &str) -> Result<()> {
    match resolve_shim_category(config, category).await? {
        ShimResolution::Found(PresetKind::Shell) => {
            Box::pin(shells::handle_uninstall(
                config,
                Some(category),
                false,
                false,
            ))
            .await
        }
        ShimResolution::Found(PresetKind::App) => {
            Box::pin(apps::handle_uninstall(
                config,
                Some(category),
                false,
                false,
                false,
            ))
            .await
        }
        ShimResolution::Conflict => match select_shim_kind("Uninstall", category)? {
            PresetKind::Shell => {
                Box::pin(shells::handle_uninstall(
                    config,
                    Some(category),
                    false,
                    false,
                ))
                .await
            }
            PresetKind::App => {
                Box::pin(apps::handle_uninstall(
                    config,
                    Some(category),
                    false,
                    false,
                    false,
                ))
                .await
            }
        },
        ShimResolution::Missing => bail_shim_missing(category),
    }
}

async fn resolve_shim_category(config: &Config, category: &str) -> Result<ShimResolution> {
    let shell_matches = if config.is_external_presets {
        let shell_path = config.presets_dir().join("shell").join(category);
        if shell_path.exists() {
            shells::metadata::load_installed_categories(config, Some(category))
                .await?
                .len()
        } else {
            0
        }
    } else {
        shells::metadata::load_embedded_categories(Some(category))?.len()
    };
    let app_matches = if config.is_external_presets {
        let app_path = config.presets_dir().join("app").join(category);
        if app_path.exists() {
            apps::load_installed_categories(config, Some(category))
                .await?
                .len()
        } else {
            0
        }
    } else {
        apps::load_embedded_categories(Some(category))?.len()
    };

    Ok(classify_shim_resolution(shell_matches > 0, app_matches > 0))
}

fn classify_shim_resolution(shell_matches: bool, app_matches: bool) -> ShimResolution {
    match (shell_matches, app_matches) {
        (true, false) => ShimResolution::Found(PresetKind::Shell),
        (false, true) => ShimResolution::Found(PresetKind::App),
        (true, true) => ShimResolution::Conflict,
        (false, false) => ShimResolution::Missing,
    }
}

fn select_shim_kind(action: &str, category: &str) -> Result<PresetKind> {
    let choices = [format!("shell/{category}"), format!("app/{category}")];
    let selected = Select::new()
        .with_prompt(format!("{action} which preset?"))
        .items(&choices)
        .default(0)
        .interact()?;
    Ok(match selected {
        0 => PresetKind::Shell,
        1 => PresetKind::App,
        _ => unreachable!("dialoguer Select returned out-of-range index {selected}"),
    })
}

fn bail_shim_missing(category: &str) -> Result<()> {
    bail!(
        "preset category not found in shell or app presets: {category}\nRun `shine shell list` or `shine app list` to see available categories."
    )
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

async fn handle_env_set(config: &Config, key: &str, value: &str) -> Result<()> {
    let mut env = env::EnvConfig::load_or_init(config).await?;
    env.set(key, value);
    env.save(config).await?;
    println!("{}", colors::green(&format!("set {key} = \"{value}\"")));
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

async fn handle_env_delete(config: &Config, key: &str) -> Result<()> {
    let mut env = env::EnvConfig::load_or_init(config).await?;
    if env.remove(key).is_none() {
        bail!("{key} is not set in the active config [env]");
    }
    env.save(config).await?;
    println!("{}", colors::green(&format!("deleted {key}")));
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

async fn handle_env_get(config: &Config, key: &str) -> Result<()> {
    let env = env::EnvConfig::load_or_init(config).await?;
    match env.get(key) {
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

async fn handle_env_decrypt(config: &Config, key: &str) -> Result<()> {
    let env = env::EnvConfig::load_or_init(config).await?;
    let Some(value) = env.get(key) else {
        bail!("{key} is not set in the active config [env]");
    };
    let plaintext = secret::decrypt_base64_gpg_secret(value)
        .await
        .with_context(|| format!("decrypting {key}"))?;
    print!("{plaintext}");
    Ok(())
}

async fn handle_env_export(config: &Config, key: &str, alias: Option<&str>) -> Result<()> {
    validate_env_export_key(key)?;
    if let Some(alias) = alias {
        validate_env_export_key(alias)?;
    }
    let env = env::EnvConfig::load_or_init(config).await?;
    let value = match resolve_env_export_value(&env, key)? {
        EnvExportValue::Secret {
            key: secret_key,
            value,
        } => secret::decrypt_base64_gpg_secret(value)
            .await
            .with_context(|| format!("decrypting {secret_key}"))?,
        EnvExportValue::Plaintext(value) => value.to_string(),
    };
    let export_as = alias.unwrap_or(key);
    println!(
        "{}",
        format_env_export(&config.shell_type, export_as, &value)
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum EnvExportValue<'a> {
    Secret { key: String, value: &'a str },
    Plaintext(&'a str),
}

fn resolve_env_export_value<'a>(env: &'a env::EnvConfig, key: &str) -> Result<EnvExportValue<'a>> {
    let secret_key = env_export_secret_key(key);
    if let Some(value) = env.get(&secret_key) {
        return Ok(EnvExportValue::Secret {
            key: secret_key,
            value,
        });
    }
    if let Some(value) = env.get(key) {
        return Ok(EnvExportValue::Plaintext(value));
    }
    bail!("{secret_key} or {key} is not set in the active config [env]");
}

fn env_export_secret_key(key: &str) -> String {
    format!("{key}_SECRET")
}

fn validate_env_export_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("env export key must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        bail!("env export key must start with a letter or underscore: {key}");
    }
    if !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        bail!("env export key must contain only letters, digits, and underscores: {key}");
    }
    Ok(())
}

fn format_env_export(shell: &shells::ShellType, key: &str, value: &str) -> String {
    match shell {
        shells::ShellType::Fish => format!("set -gx {key} {}", fish_quote(value)),
        shells::ShellType::PowerShell => {
            format!("$env:{key} = {}", powershell_string_quote(value))
        }
        _ => format!("export {key}={}", posix_shell_quote(value)),
    }
}

fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn powershell_string_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[derive(Debug, PartialEq, Eq)]
enum EnvEncryptOutput {
    Print,
    Set(String),
}

fn resolve_env_encrypt_output(
    set_key: Option<&str>,
    from_key: Option<&str>,
) -> Result<EnvEncryptOutput> {
    if let Some(key) = set_key {
        return Ok(EnvEncryptOutput::Set(key.to_string()));
    }
    if let Some(key) = from_key {
        validate_env_export_key(key)?;
        return Ok(EnvEncryptOutput::Set(env_export_secret_key(key)));
    }
    Ok(EnvEncryptOutput::Print)
}

fn resolve_env_encrypt_recipient(config: &Config, recipient: Option<&str>) -> Result<String> {
    if let Some(recipient) = recipient
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
    {
        return Ok(recipient.to_string());
    }
    if let Some(recipient) = config
        .gpg_key_id
        .as_deref()
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
    {
        return Ok(recipient.to_string());
    }
    bail!("GPG recipient is required; pass -r/--recipient or set gpg_key_id in config.toml");
}

async fn handle_env_encrypt(
    config: &Config,
    recipient: Option<&str>,
    set_key: Option<&str>,
    from_key: Option<&str>,
) -> Result<()> {
    use std::io::Read as _;

    let recipient = resolve_env_encrypt_recipient(config, recipient)?;
    let plaintext = if let Some(key) = from_key {
        let env = env::EnvConfig::load_or_init(config).await?;
        let Some(value) = env.get(key) else {
            bail!("{key} is not set in the active config [env]");
        };
        value.as_bytes().to_vec()
    } else {
        let mut input = Vec::new();
        std::io::stdin()
            .read_to_end(&mut input)
            .context("reading secret from stdin")?;
        input
    };
    let encoded = secret::encrypt_gpg_secret_to_base64(&plaintext, &recipient)
        .await
        .with_context(|| format!("encrypting secret for {recipient}"))?;
    match resolve_env_encrypt_output(set_key, from_key)? {
        EnvEncryptOutput::Set(key) => {
            let mut env = env::EnvConfig::load_or_init(config).await?;
            env.set(&key, &encoded);
            env.save(config).await?;
            println!("{}", colors::green(&format!("set {key} = \"{encoded}\"")));
        }
        EnvEncryptOutput::Print => println!("{encoded}"),
    }
    Ok(())
}

async fn handle_update(config: &Config, verbose: bool, refresh: bool) -> Result<()> {
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

    let update_status = if refresh {
        update_check::check_for_update_forced(config).await
    } else {
        update_check::check_for_update(config).await
    };

    match update_status {
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
            eprintln!("{}", format_update_check_failure_warning(&e));
        }
    }

    if !printed_update {
        println!("{}", colors::dim("Nothing to update."));
    }

    Ok(())
}

fn format_update_check_failure_warning(err: &anyhow::Error) -> String {
    colors::yellow(&format!("warning: skipped shine version check: {err}"))
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
            previous_display,
            release_tag,
            installed_version,
            installed_path,
        }) => {
            println!(
                "{}",
                colors::green(&format_self_upgrade_message(
                    channel,
                    &previous_display,
                    &installed_version,
                    &release_tag,
                ))
            );
            sync_self_install_dest(config, &installed_path).await;
        }
        Err(e) => {
            update_check::invalidate_update_cache(config).await;
            bail!("Upgrade failed: {e}");
        }
    }

    Ok(())
}

fn format_self_upgrade_message(
    channel: ReleaseChannel,
    previous_display: &str,
    installed_version: &str,
    release_tag: &str,
) -> String {
    match channel {
        ReleaseChannel::Stable => {
            format!("Upgraded shine from {previous_display} to {installed_version}.")
        }
        ReleaseChannel::Preview => {
            if previous_display.contains("+preview.") {
                format!(
                    "Updated shine preview from {previous_display} to {installed_version} ({release_tag})."
                )
            } else {
                format!(
                    "Installed shine preview {installed_version} over stable {previous_display} ({release_tag})."
                )
            }
        }
    }
}

async fn handle_config_upgrade(config: &Config, verbose: bool, prune_stale: bool) -> Result<()> {
    println!("{}", colors::bold("Upgrading installed configs"));
    crate::config::print_presets_note(config);

    let env_report = Box::pin(env::upgrade::handle_upgrade(config, false, verbose)).await?;
    let shell_report = Box::pin(shells::handle_upgrade_installed(config, verbose)).await?;
    let app_report = Box::pin(apps::handle_upgrade_installed(config, prune_stale)).await?;

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
    output::footer("Done", &summary);

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
        Err(e) if has_io_error_kind(&e, std::io::ErrorKind::PermissionDenied) => {
            let hint = if cfg!(windows) {
                format!(
                    "Installed copy at {} needs manual sync; rerun from an elevated terminal if needed.",
                    dest.display()
                )
            } else {
                format!(
                    "System copy at {} needs manual sync — run: sudo {} self install",
                    dest.display(),
                    src.display()
                )
            };
            println!("{}", colors::yellow(&hint));
        }
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

async fn handle_overlay_link(config: &Config, path: PathBuf, create: bool) -> Result<()> {
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
        .presets_overlay_dir_override
        .as_deref()
        .is_some_and(|p| p == absolute)
    {
        println!(
            "{}",
            colors::dim(&format!("overlay already linked: {}", absolute.display()))
        );
        return Ok(());
    }

    let updated = config
        .clone()
        .with_presets_overlay_dir_override(Some(absolute.clone()));
    updated.save().await?;

    println!("{}", colors::presets_overlay_note(&absolute));
    if config.is_external_presets {
        println!(
            "{}",
            colors::yellow(
                "Warning: a full external presets source is active, so this overlay is configured but not used."
            )
        );
    } else {
        println!(
            "{}",
            colors::dim("Overlay files override built-in presets by matching path.")
        );
    }

    Ok(())
}

async fn handle_overlay_unlink(config: &Config) -> Result<()> {
    if config.presets_overlay_dir_override.is_none() {
        println!(
            "{}",
            colors::dim("No presets overlay directory is configured.")
        );
        return Ok(());
    }

    let updated = config.clone().with_presets_overlay_dir_override(None);
    updated.save().await?;

    println!(
        "{}",
        colors::green("Presets overlay directory removed from the active config.")
    );
    println!(
        "{}",
        colors::dim("Built-in embedded presets will be used without overlay on the next run.")
    );

    Ok(())
}

fn handle_overlay_show(config: &Config) -> Result<()> {
    if let Some(dir) = &config.presets_overlay_dir_override {
        println!("{}", colors::presets_overlay_note(dir));
        if config.is_external_presets {
            println!(
                "{}",
                colors::yellow(
                    "Inactive: a full external presets source is active and takes priority."
                )
            );
        } else {
            println!("{}", colors::green("Active"));
        }
    } else {
        println!(
            "{}",
            colors::dim("No presets overlay directory is configured.")
        );
    }
    Ok(())
}

async fn handle_self_install(mut config: Config, dest: Option<std::path::PathBuf>) -> Result<()> {
    use anyhow::{Context as _, bail};

    let src = std::env::current_exe().context("failed to resolve current executable path")?;
    let dest = match dest {
        Some(dest) => dest,
        None => platform::default_self_install_dest()?,
    };

    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.clone());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.clone());
        if canonical_src == canonical_dest {
            let example = if cfg!(windows) {
                r"C:\path\to\new\shine.exe self install"
            } else {
                "sudo /path/to/new/shine self install"
            };
            bail!(
                "source and destination are the same binary: {}. Run the newer binary by full path, e.g. `{example}`, to overwrite this copy.",
                dest.display()
            );
        }
    }

    install_binary_atomically(&src, &dest)
        .with_context(|| self_install_failure_hint(&src, &dest))?;

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
    print_self_install_activation_hint(&dest);

    Ok(())
}

fn self_install_failure_hint(src: &std::path::Path, dest: &std::path::Path) -> String {
    if cfg!(windows) {
        format!("failed to copy to {}", dest.display())
    } else {
        format!(
            "failed to copy to {} — try: sudo {} self install",
            dest.display(),
            src.display()
        )
    }
}

fn print_self_install_activation_hint(dest: &std::path::Path) {
    let Some(dir) = dest.parent() else {
        return;
    };
    if platform::current_path_contains_dir(dir) {
        println!(
            "{}",
            colors::dim("The install directory is already on PATH.")
        );
    } else {
        println!(
            "{}",
            colors::yellow(&format!(
                "Install directory is not on PATH: {}",
                dir.display()
            ))
        );
        println!("{}", colors::dim(&platform::path_install_hint(dir)));
    }
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
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set permissions on {}", temp.display()))?;
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
    use crate::test_support::env_lock;
    use tokio::fs;

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

        let err = handle_self_install(config, Some(current))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("source and destination are the same binary"),
            "error should explain self-overwrite: {err:#}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn cli_accepts_refactored_update_commands() {
        let cli = Cli::try_parse_from(["shine", "self", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Self_ {
                command: SelfCommands::Install { dest: None }
            }
        ));

        let cli =
            Cli::try_parse_from(["shine", "self", "install", "--dest", "/tmp/shine"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Self_ {
                command: SelfCommands::Install { dest: Some(ref path) }
            } if path.as_path() == std::path::Path::new("/tmp/shine")
        ));

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
            Commands::Update(UpdateCommand {
                verbose: false,
                refresh: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                verbose: true,
                refresh: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--refresh"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                verbose: false,
                refresh: true
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                verbose: false,
                prune_stale: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                verbose: true,
                prune_stale: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--prune-stale"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                verbose: false,
                prune_stale: true
            })
        ));

        let cli = Cli::try_parse_from(["shine", "clear"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Clear(ClearCommand { dry_run: false })
        ));

        let cli = Cli::try_parse_from(["shine", "clear", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Clear(ClearCommand { dry_run: true })
        ));
    }

    #[test]
    fn update_check_failure_warning_is_non_fatal_wording() {
        let err = anyhow::anyhow!(
            "GitHub stable release request failed: HTTP 403 Forbidden: API rate limit exceeded"
        );
        let warning = format_update_check_failure_warning(&err);

        assert!(warning.contains("warning: skipped shine version check"));
        assert!(warning.contains("HTTP 403 Forbidden"));
        assert!(!warning.contains("Update check failed"));
    }

    #[test]
    fn format_self_upgrade_message_handles_stable_channel() {
        assert_eq!(
            format_self_upgrade_message(ReleaseChannel::Stable, "0.21.3", "0.21.4", "v0.21.4",),
            "Upgraded shine from 0.21.3 to 0.21.4."
        );
    }

    #[test]
    fn format_self_upgrade_message_handles_stable_to_preview_install() {
        assert_eq!(
            format_self_upgrade_message(
                ReleaseChannel::Preview,
                "0.21.3",
                "0.21.4+preview.237a8a0",
                "preview",
            ),
            "Installed shine preview 0.21.4+preview.237a8a0 over stable 0.21.3 (preview)."
        );
    }

    #[test]
    fn format_self_upgrade_message_handles_preview_to_preview_update() {
        assert_eq!(
            format_self_upgrade_message(
                ReleaseChannel::Preview,
                "0.21.4+preview.1111111",
                "0.21.4+preview.237a8a0",
                "preview",
            ),
            "Updated shine preview from 0.21.4+preview.1111111 to 0.21.4+preview.237a8a0 (preview)."
        );
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
    fn cli_accepts_overlay_commands() {
        let cli = Cli::try_parse_from(["shine", "overlay", "link", "/tmp/presets"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Overlay {
                command: OverlayCommands::Link(LinkCommand {
                    ref path,
                    create: false
                })
            } if path.as_path() == std::path::Path::new("/tmp/presets")
        ));

        let cli =
            Cli::try_parse_from(["shine", "overlay", "link", "/tmp/presets", "--create"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Overlay {
                command: OverlayCommands::Link(LinkCommand {
                    ref path,
                    create: true
                })
            } if path.as_path() == std::path::Path::new("/tmp/presets")
        ));

        assert!(Cli::try_parse_from(["shine", "overlay", "unlink"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "overlay", "show"]).is_ok());
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
    fn cli_accepts_env_export_command() {
        let cli = Cli::try_parse_from(["shine", "env", "export", "DEEPSEEK_API_KEY"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Export { key, alias: None }
            } if key == "DEEPSEEK_API_KEY"
        ));
    }

    #[test]
    fn cli_accepts_env_export_with_alias() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "export",
            "DEEPSEEK_API_KEY",
            "--as",
            "AI_KEY",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Export { key, alias: Some(a) }
            } if key == "DEEPSEEK_API_KEY" && a == "AI_KEY"
        ));
    }

    #[test]
    fn env_export_uses_alias_as_variable_name() {
        let value = "secret123";
        assert_eq!(
            format_env_export(&shells::ShellType::Zsh, "MY_ALIAS", value),
            "export MY_ALIAS='secret123'"
        );
    }

    #[test]
    fn env_export_alias_formats_powershell_correctly() {
        let value = "secret123";
        assert_eq!(
            format_env_export(&shells::ShellType::PowerShell, "MY_ALIAS", value),
            "$env:MY_ALIAS = 'secret123'"
        );
    }

    #[test]
    fn cli_accepts_env_delete_command() {
        let cli = Cli::try_parse_from(["shine", "env", "delete", "MY_TOKEN"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Delete { key }
            } if key == "MY_TOKEN"
        ));
    }

    #[test]
    fn cli_accepts_env_encrypt_without_recipient() {
        let cli = Cli::try_parse_from(["shine", "env", "encrypt", "--from", "MY_TOKEN"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Encrypt(cmd)
            } if cmd.recipient.is_none() && cmd.from.as_deref() == Some("MY_TOKEN")
        ));
    }

    #[test]
    fn cli_accepts_env_encrypt_with_recipient() {
        let cli =
            Cli::try_parse_from(["shine", "env", "encrypt", "-r", "alice@example.com"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Encrypt(cmd)
            } if cmd.recipient.as_deref() == Some("alice@example.com")
        ));
    }

    #[tokio::test]
    async fn env_delete_removes_key_from_saved_config() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.env.insert("MY_TOKEN".into(), "secret".into());
        config.save().await.unwrap();

        handle_env_delete(&config, "MY_TOKEN").await.unwrap();

        let contents = fs::read_to_string(config.config_path()).await.unwrap();
        let parsed: toml::Table = toml::from_str(&contents).unwrap();
        let env = parsed
            .get("env")
            .and_then(|value| value.as_table())
            .unwrap();
        assert!(
            !env.contains_key("MY_TOKEN"),
            "deleted key should not remain in saved config: {contents}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_delete_fails_when_key_is_missing() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let err = handle_env_delete(&config, "MY_TOKEN").await.unwrap_err();

        assert!(
            err.to_string()
                .contains("MY_TOKEN is not set in the active config [env]"),
            "error should explain missing key: {err:#}"
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn env_export_secret_key_appends_secret_suffix() {
        assert_eq!(
            env_export_secret_key("DEEPSEEK_API_KEY"),
            "DEEPSEEK_API_KEY_SECRET"
        );
        assert_eq!(env_export_secret_key("xxx"), "xxx_SECRET");
    }

    #[test]
    fn env_export_resolves_secret_when_present() {
        let mut env = env::EnvConfig::default();
        env.set("MY_TOKEN_SECRET", "encrypted");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Secret {
                key: "MY_TOKEN_SECRET".to_string(),
                value: "encrypted"
            }
        );
    }

    #[test]
    fn env_export_falls_back_to_plaintext_value() {
        let mut env = env::EnvConfig::default();
        env.set("MY_TOKEN", "plain");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Plaintext("plain")
        );
    }

    #[test]
    fn env_export_secret_wins_over_plaintext_value() {
        let mut env = env::EnvConfig::default();
        env.set("MY_TOKEN", "plain");
        env.set("MY_TOKEN_SECRET", "encrypted");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Secret {
                key: "MY_TOKEN_SECRET".to_string(),
                value: "encrypted"
            }
        );
    }

    #[test]
    fn env_export_reports_both_missing_keys() {
        let env = env::EnvConfig::default();

        let err = resolve_env_export_value(&env, "MY_TOKEN").unwrap_err();

        assert!(
            err.to_string()
                .contains("MY_TOKEN_SECRET or MY_TOKEN is not set in the active config [env]"),
            "error should explain both checked keys: {err:#}"
        );
    }

    #[test]
    fn env_export_key_validation_accepts_shell_variable_names() {
        for key in ["FOO", "_FOO", "foo_123", "A1"] {
            validate_env_export_key(key).unwrap();
        }
    }

    #[test]
    fn env_export_key_validation_rejects_unsafe_names() {
        for key in ["", "1FOO", "FOO-BAR", "FOO;BAR", "FOO BAR", "FOO.SECRET"] {
            assert!(
                validate_env_export_key(key).is_err(),
                "key should be rejected: {key}"
            );
        }
    }

    #[test]
    fn env_export_formats_posix_shell_code_safely() {
        let value = "abc def'ghi$HOME\nnext; rm -rf /";
        assert_eq!(
            format_env_export(&shells::ShellType::Zsh, "TOKEN", value),
            "export TOKEN='abc def'\\''ghi$HOME\nnext; rm -rf /'"
        );
    }

    #[test]
    fn env_export_formats_fish_shell_code_safely() {
        let value = "abc def'ghi\\path\nnext; rm -rf /";
        assert_eq!(
            format_env_export(&shells::ShellType::Fish, "TOKEN", value),
            "set -gx TOKEN 'abc def\\'ghi\\\\path\nnext; rm -rf /'"
        );
    }

    #[test]
    fn env_export_formats_powershell_code_safely() {
        let value = "abc def'ghi$HOME\nnext; Remove-Item /";
        assert_eq!(
            format_env_export(&shells::ShellType::PowerShell, "TOKEN", value),
            "$env:TOKEN = 'abc def''ghi$HOME\nnext; Remove-Item /'"
        );
    }

    #[test]
    fn env_encrypt_output_defaults_from_key_to_secret_key() {
        assert_eq!(
            resolve_env_encrypt_output(None, Some("GH_TOKEN")).unwrap(),
            EnvEncryptOutput::Set("GH_TOKEN_SECRET".to_string())
        );
    }

    #[test]
    fn env_encrypt_output_explicit_set_wins_over_default() {
        assert_eq!(
            resolve_env_encrypt_output(Some("CUSTOM_SECRET"), Some("GH_TOKEN")).unwrap(),
            EnvEncryptOutput::Set("CUSTOM_SECRET".to_string())
        );
    }

    #[test]
    fn env_encrypt_output_prints_stdin_without_set() {
        assert_eq!(
            resolve_env_encrypt_output(None, None).unwrap(),
            EnvEncryptOutput::Print
        );
    }

    #[test]
    fn env_encrypt_output_rejects_invalid_inferred_from_key() {
        let err = resolve_env_encrypt_output(None, Some("GH-TOKEN")).unwrap_err();

        assert!(
            err.to_string()
                .contains("env export key must contain only letters, digits, and underscores"),
            "error should explain invalid inferred key: {err:#}"
        );
    }

    #[test]
    fn env_encrypt_recipient_cli_wins_over_config() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("config@example.com".to_string());

        assert_eq!(
            resolve_env_encrypt_recipient(&config, Some("cli@example.com")).unwrap(),
            "cli@example.com"
        );
    }

    #[test]
    fn env_encrypt_recipient_falls_back_to_config() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("config@example.com".to_string());

        assert_eq!(
            resolve_env_encrypt_recipient(&config, None).unwrap(),
            "config@example.com"
        );
    }

    #[test]
    fn env_encrypt_recipient_treats_empty_config_as_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("  ".to_string());

        let err = resolve_env_encrypt_recipient(&config, None).unwrap_err();

        assert!(
            err.to_string()
                .contains("pass -r/--recipient or set gpg_key_id"),
            "error should explain how to set recipient: {err:#}"
        );
    }

    #[test]
    fn env_encrypt_recipient_errors_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        let err = resolve_env_encrypt_recipient(&config, None).unwrap_err();

        assert!(
            err.to_string()
                .contains("pass -r/--recipient or set gpg_key_id"),
            "error should explain how to set recipient: {err:#}"
        );
    }

    #[test]
    fn cli_accepts_top_level_install_reinstall_and_uninstall_commands() {
        let cli = Cli::try_parse_from(["shine", "install", "proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Install { category } if category == "proxy"
        ));

        let cli = Cli::try_parse_from(["shine", "reinstall", "proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Reinstall { category } if category == "proxy"
        ));

        let cli = Cli::try_parse_from(["shine", "uninstall", "starship"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Uninstall { category } if category == "starship"
        ));
    }

    #[test]
    fn cli_rejects_top_level_shims_without_category() {
        assert!(Cli::try_parse_from(["shine", "install"]).is_err());
        assert!(Cli::try_parse_from(["shine", "reinstall"]).is_err());
        assert!(Cli::try_parse_from(["shine", "uninstall"]).is_err());
    }

    #[test]
    fn classify_shim_resolution_handles_all_match_shapes() {
        assert_eq!(
            classify_shim_resolution(true, false),
            ShimResolution::Found(PresetKind::Shell)
        );
        assert_eq!(
            classify_shim_resolution(false, true),
            ShimResolution::Found(PresetKind::App)
        );
        assert_eq!(
            classify_shim_resolution(true, true),
            ShimResolution::Conflict
        );
        assert_eq!(
            classify_shim_resolution(false, false),
            ShimResolution::Missing
        );
    }

    #[tokio::test]
    async fn resolve_shim_category_matches_embedded_shell_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "proxy").await.unwrap();

        assert_eq!(resolution, ShimResolution::Found(PresetKind::Shell));
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_shim_category_matches_embedded_app_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "starship").await.unwrap();

        assert_eq!(resolution, ShimResolution::Found(PresetKind::App));
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_shim_category_reports_missing_category() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let resolution = resolve_shim_category(&config, "does-not-exist")
            .await
            .unwrap();

        assert_eq!(resolution, ShimResolution::Missing);
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn runtime_schema_warning_is_skipped_for_lifecycle_commands() {
        let init = Commands::Init(InitCommand { yes: false });
        let completions = Commands::Completions {
            command: CompletionCommands::Bash,
        };
        let clear = Commands::Clear(ClearCommand { dry_run: false });
        let list = Commands::List;

        assert!(!should_warn_runtime_schema(&init));
        assert!(!should_warn_runtime_schema(&completions));
        assert!(!should_warn_runtime_schema(&clear));
        assert!(should_warn_runtime_schema(&list));
    }

    #[test]
    fn cli_accepts_info_command() {
        let cli = Cli::try_parse_from(["shine", "info", "setproxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: false,
                verbose: false
            } if target == "setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "info", "setproxy", "--diff"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: true,
                verbose: false
            } if target == "setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "info", "setproxy", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: false,
                verbose: true
            } if target == "setproxy"
        ));

        let cli =
            Cli::try_parse_from(["shine", "info", "setproxy", "--diff", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: true,
                verbose: true
            } if target == "setproxy"
        ));
    }

    #[test]
    fn cli_rejects_legacy_show_command() {
        let err = Cli::try_parse_from(["shine", "show", "setproxy"]).unwrap_err();
        assert!(err.to_string().contains("unrecognized subcommand 'show'"));
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
    fn cli_accepts_shell_and_app_reinstall_commands() {
        let cli = Cli::try_parse_from(["shine", "shell", "reinstall", "proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Reinstall { category }
            } if category.as_deref() == Some("proxy")
        ));

        let cli = Cli::try_parse_from(["shine", "app", "reinstall", "ghostty"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Reinstall {
                    category,
                    dry_run: false
                }
            } if category.as_deref() == Some("ghostty")
        ));

        let cli =
            Cli::try_parse_from(["shine", "app", "reinstall", "ghostty", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Reinstall {
                    category,
                    dry_run: true
                }
            } if category.as_deref() == Some("ghostty")
        ));
    }

    #[test]
    fn cli_rejects_install_force_options() {
        let err =
            Cli::try_parse_from(["shine", "shell", "install", "proxy", "--force"]).unwrap_err();
        assert!(err.to_string().contains("unexpected argument '--force'"));

        let err =
            Cli::try_parse_from(["shine", "app", "install", "ghostty", "--force"]).unwrap_err();
        assert!(err.to_string().contains("unexpected argument '--force'"));
    }

    #[test]
    fn cli_accepts_sys_init_options() {
        let cli = Cli::try_parse_from(["shine", "sys", "init"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: None,
                    dry_run: false,
                    force_profile: false
                }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "init", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: None,
                    dry_run: true,
                    force_profile: false
                }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "init", "--preset", "recommended"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: Some(ref preset),
                    dry_run: false,
                    force_profile: false
                }
            } if preset == "recommended"
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "init", "--force-profile"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: None,
                    dry_run: false,
                    force_profile: true
                }
            }
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "sys",
            "init",
            "--preset",
            "recommended",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: Some(ref preset),
                    dry_run: true,
                    force_profile: false
                }
            } if preset == "recommended"
        ));
    }

    #[test]
    fn cli_accepts_sys_status() {
        let cli = Cli::try_parse_from(["shine", "sys", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Status
            }
        ));
    }

    #[test]
    fn cli_completions_rejects_unsupported_shells() {
        assert!(Cli::try_parse_from(["shine", "completions", "elvish"]).is_err());
        assert!(Cli::try_parse_from(["shine", "completions", "fish"]).is_err());
    }

    #[test]
    fn cli_completions_accepts_supported_commands() {
        assert!(Cli::try_parse_from(["shine", "completions", "install"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "completions", "bash"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "completions", "powershell"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "completions", "zsh"]).is_ok());
    }

    #[test]
    fn completions_output_is_non_empty_for_supported_shells() {
        for shell in [
            CompletionShell::Bash,
            CompletionShell::PowerShell,
            CompletionShell::Zsh,
        ] {
            let mut command = completion::command();
            let mut output = Vec::new();

            match shell {
                CompletionShell::Bash => clap_complete::generate(
                    clap_complete::shells::Bash,
                    &mut command,
                    "shine",
                    &mut output,
                ),
                CompletionShell::PowerShell => clap_complete::generate(
                    clap_complete::shells::PowerShell,
                    &mut command,
                    "shine",
                    &mut output,
                ),
                CompletionShell::Zsh => clap_complete::generate(
                    clap_complete::shells::Zsh,
                    &mut command,
                    "shine",
                    &mut output,
                ),
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

        // SAFETY: `_guard` holds `env_lock()`, serialising SHINE_PRESETS mutations across test threads.
        unsafe { std::env::set_var("SHINE_PRESETS", "/some/override") };
        // Should succeed even with env var set
        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();
        // SAFETY: `_guard` holds `env_lock()`, serialising SHINE_PRESETS mutations across test threads.
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
