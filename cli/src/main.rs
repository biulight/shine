use anyhow::{Context, Result, bail};
use clap::Parser;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use cli::test_support;
use cli::{
    apps, clear, colors, commands, completion, config, env, git_pull, list, shells, show, ssh, sys,
    update_check, version,
};

use commands::{
    AppCommands, Cli, Commands, CompletionCommands, CompletionShell, EnvCommands,
    EnvIdentitySubcommand, ExportCommand, LinkCommand, LocalCommands, OverlayCommands,
    SelfCommands, ShellCommands, SysCommands,
};
#[cfg(test)]
use commands::{ClearCommand, InitCommand, UpdateCommand, UpgradeCommand};
use config::Config;
#[cfg(test)]
use update_check::ReleaseChannel;
use update_check::UpdateStatus;

mod presets_commands;
mod self_install;
mod shim;

use presets_commands::{
    handle_overlay_link, handle_overlay_show, handle_overlay_unlink, handle_presets_export,
    handle_presets_link, handle_presets_unlink,
};
#[cfg(test)]
use self_install::{
    SelfInstallSync, format_self_upgrade_message, format_update_check_failure_warning,
    install_binary_atomically, sync_self_install_dest_from,
};
use self_install::{
    handle_config_upgrade, handle_self_install, handle_self_upgrade, handle_update,
};
use shim::{handle_install_shim, handle_reinstall_shim, handle_uninstall_shim};

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
    let skip_background_update_check = matches!(
        &cli.command,
        Commands::Update(..)
            | Commands::Pull
            | Commands::Export(..)
            | Commands::Link(..)
            | Commands::Unlink
            | Commands::Overlay { .. }
            | Commands::Clear(..)
            | Commands::Self_ { .. }
            | Commands::Env { .. }
    ) || matches!(&cli.command, Commands::Upgrade(cmd) if cmd.pull);
    if !skip_background_update_check {
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
        Commands::Pull => git_pull::handle_pull(&config, false).await,
        Commands::Update(cmd) => {
            if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                let config = Box::pin(Config::load_or_init()).await?;
                handle_update(&config, cmd.verbose, cmd.refresh).await
            } else {
                handle_update(&config, cmd.verbose, cmd.refresh).await
            }
        }
        Commands::Upgrade(cmd) => {
            if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                let config = Box::pin(Config::load_or_init()).await?;
                handle_config_upgrade(&config, cmd.verbose, cmd.prune_stale).await
            } else {
                handle_config_upgrade(&config, cmd.verbose, cmd.prune_stale).await
            }
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
            EnvCommands::Show { reveal } => env::commands::handle_show(&config, reveal).await,
            EnvCommands::Set { key, value } => {
                env::commands::handle_set(&config, &key, &value).await
            }
            EnvCommands::Delete { key } => env::commands::handle_delete(&config, &key).await,
            EnvCommands::Get { key } => env::commands::handle_get(&config, &key).await,
            EnvCommands::Decrypt { key } => env::commands::handle_decrypt(&config, &key).await,
            EnvCommands::Export { key, alias } => {
                env::commands::handle_export(&config, &key, alias.as_deref()).await
            }
            EnvCommands::Encrypt(cmd) => {
                env::commands::handle_encrypt(
                    &config,
                    cmd.backend.as_deref(),
                    &cmd.recipients,
                    cmd.set.as_deref(),
                    cmd.from.as_deref(),
                )
                .await
            }
            EnvCommands::Seal(cmd) => {
                env::workspace::handle_seal(
                    &config,
                    cmd.workspace.as_deref(),
                    cmd.file.as_deref(),
                    cmd.backend.as_deref(),
                    &cmd.recipients,
                )
                .await
            }
            EnvCommands::Run(cmd) => {
                env::workspace::handle_run(
                    &config,
                    cmd.workspace.as_deref(),
                    cmd.mode.as_deref(),
                    &cmd.with,
                    &cmd.command,
                )
                .await
            }
            EnvCommands::Identity(cmd) => match cmd.command {
                EnvIdentitySubcommand::Init {
                    touch_id,
                    access_control,
                    output,
                    force,
                } => {
                    env::identity::handle_identity_init(
                        &config,
                        touch_id,
                        access_control.as_deref(),
                        output.as_deref(),
                        force,
                    )
                    .await
                }
                EnvIdentitySubcommand::Show => env::identity::handle_identity_show(&config).await,
            },
        },
        Commands::Sys { command } => match command {
            SysCommands::List { all } => Box::pin(sys::handle_list(&config, all)).await,
            SysCommands::Info { item } => Box::pin(sys::handle_info(&config, &item)).await,
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
            SysCommands::Apply { item, dry_run } => {
                Box::pin(sys::handle_apply(&config, item.as_deref(), dry_run)).await
            }
            SysCommands::Uninstall { item, dry_run } => {
                Box::pin(sys::handle_uninstall(&config, &item, dry_run)).await
            }
        },
        Commands::Ssh { args } => ssh::handle_ssh(&config, &args).await,
        Commands::Local { command } => match command {
            LocalCommands::Download(cmd) => {
                ssh::handle_local_download(
                    &cmd.source,
                    cmd.destination.as_deref(),
                    cmd.force,
                    cmd.dry_run,
                )
                .await
            }
            LocalCommands::Upload(cmd) => {
                ssh::handle_local_upload(
                    &cmd.source,
                    cmd.destination.as_deref(),
                    cmd.force,
                    cmd.dry_run,
                )
                .await
            }
            LocalCommands::Status => ssh::handle_local_status().await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::env_lock;
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
                pull: false,
                verbose: false,
                refresh: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                pull: false,
                verbose: true,
                refresh: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--refresh"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                pull: false,
                verbose: false,
                refresh: true
            })
        ));

        let cli = Cli::try_parse_from(["shine", "pull"]).unwrap();
        assert!(matches!(cli.command, Commands::Pull));

        let cli = Cli::try_parse_from(["shine", "update", "--pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                pull: true,
                verbose: false,
                refresh: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                pull: false,
                verbose: false,
                prune_stale: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                pull: false,
                verbose: true,
                prune_stale: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--prune-stale"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                pull: false,
                verbose: false,
                prune_stale: true
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                pull: true,
                verbose: false,
                prune_stale: false
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
    fn cli_accepts_env_show_reveal() {
        let cli = Cli::try_parse_from(["shine", "env", "show", "--reveal"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Show { reveal: true }
            }
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
            } if cmd.recipients.is_empty() && cmd.from.as_deref() == Some("MY_TOKEN")
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
            } if cmd.recipients == ["alice@example.com"]
        ));
    }

    #[test]
    fn cli_accepts_workspace_env_seal() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "seal",
            ".env.production.shine.toml",
            "--recipient",
            "alice@example.com",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Seal(cmd)
            } if cmd.file.as_deref() == Some(std::path::Path::new(".env.production.shine.toml"))
                && cmd.recipients == ["alice@example.com"]
        ));
    }

    #[test]
    fn cli_accepts_workspace_env_run_trailing_command() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "run",
            "--mode",
            "production",
            "--",
            "bun",
            "run",
            "build",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Run(cmd)
            } if cmd.mode.as_deref() == Some("production")
                && cmd.with.is_empty()
                && cmd.command == ["bun", "run", "build"]
        ));
    }

    #[test]
    fn cli_accepts_env_run_with_multiple_explicit_values() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "run",
            "--with",
            "TOKEN_A",
            "--with",
            "TOKEN_B=OTHER_TOKEN",
            "--",
            "bun",
            "run",
            "build",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Run(cmd)
            } if cmd.with == ["TOKEN_A", "TOKEN_B=OTHER_TOKEN"]
                && cmd.command == ["bun", "run", "build"]
        ));
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
    fn cli_accepts_sys_list_and_info() {
        let cli = Cli::try_parse_from(["shine", "sys", "list", "--all"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::List { all: true }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "info", "split-dns"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Info { ref item }
            } if item == "split-dns"
        ));
    }

    #[test]
    fn cli_accepts_sys_apply_and_uninstall() {
        let cli = Cli::try_parse_from(["shine", "sys", "apply", "split-dns", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Apply {
                    item: Some(ref item),
                    dry_run: true
                }
            } if item == "split-dns"
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "uninstall", "split-dns"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Uninstall {
                    ref item,
                    dry_run: false
                }
            } if item == "split-dns"
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
