use anyhow::{Context, Result, bail};
use clap::Parser;

use cli::{
    apps, colors, commands, completion, config, env, git_pull, info, list, serve, shells, ssh,
    state, sys, task, theme, update_check,
};

use commands::{
    AppCommands, Cli, Commands, CompletionCommands, CompletionShell, EnvCommands,
    EnvIdentitySubcommand, LocalCommands, OverlayCommands, PresetCommands, SelfCommands,
    ServeCommands, ShellCommands, StateCommands, SysCommands, TaskCommands, ThemeCommands,
};
#[cfg(test)]
use commands::{
    InitCommand, OverlayLinkCommand, RemoteShell, StateMigrateCommand, UpdateCommand,
    UpgradeCommand,
};
use config::Config;
#[cfg(test)]
use update_check::ReleaseChannel;

use cli::preset_commands::{
    handle_overlay_info, handle_overlay_link, handle_overlay_unlink, handle_preset_copy,
    handle_preset_export, handle_preset_link, handle_preset_unlink,
};
use cli::self_install::{
    handle_config_upgrade, handle_self_install, handle_self_upgrade, handle_update,
};
use cli::shim::{handle_install_shim, handle_reinstall_shim, handle_uninstall_shim};

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
        return cli::init::handle_init(cmd.yes).await;
    }

    if let Commands::State {
        command: StateCommands::Migrate(cmd),
    } = &cli.command
    {
        let config = if cmd.dry_run {
            Box::pin(Config::load_global_runtime_for_dry_run()).await?
        } else {
            Box::pin(Config::load_global_runtime_or_init()).await?
        };
        return Box::pin(state::handle_migrate(&config, cmd.dry_run)).await;
    }

    // Bypassed like Init/State above: this runs on every interactive shell
    // start (from the managed profile), so it must skip Config::load_or_init()
    // (which writes to disk), the runtime-schema warning, and the background
    // update check entirely — not just opt out of them individually.
    // theme::handle_sync does its own read-only config load.
    if let Commands::Theme {
        command: ThemeCommands::Sync { auto, quiet },
    } = &cli.command
    {
        return theme::handle_sync(*auto, *quiet).await;
    }

    let config = Box::pin(Config::load_or_init()).await?;

    warn_if_runtime_schema_pending(&cli.command).await;

    update_check::maybe_notify(&config, &cli.command).await?;

    match cli.command {
        Commands::Init(_) => unreachable!(),
        Commands::Completions {
            command: CompletionCommands::Install,
        } => Box::pin(shells::handle_completion_install(&config)).await,
        Commands::Completions { .. } => unreachable!(),
        Commands::State { .. } => unreachable!(),
        Commands::Theme { .. } => unreachable!(),
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
            AppCommands::Refresh {
                category,
                file,
                force,
            } => {
                Box::pin(apps::handle_refresh(
                    &config,
                    &category,
                    file.as_deref(),
                    force,
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
            AppCommands::Build { app_id } => Box::pin(apps::handle_build(&config, &app_id)).await,
            AppCommands::Unbuild { app_id } => {
                Box::pin(apps::handle_unbuild(&config, &app_id)).await
            }
        },
        Commands::Update(cmd) => {
            if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                let config = Box::pin(Config::load_or_init()).await?;
                handle_update(
                    &config,
                    cmd.target.as_deref(),
                    cmd.diff,
                    cmd.verbose,
                    cmd.refresh_release,
                )
                .await
            } else {
                handle_update(
                    &config,
                    cmd.target.as_deref(),
                    cmd.diff,
                    cmd.verbose,
                    cmd.refresh_release,
                )
                .await
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
        Commands::Preset { command } => match command {
            PresetCommands::Export(cmd) => {
                Box::pin(handle_preset_export(&config, cmd.dir, cmd.force)).await
            }
            PresetCommands::Copy(cmd) => Box::pin(handle_preset_copy(&cmd.target, cmd.force)).await,
            PresetCommands::Link(cmd) => {
                Box::pin(handle_preset_link(&config, cmd.path, cmd.create)).await
            }
            PresetCommands::Unlink => Box::pin(handle_preset_unlink(&config)).await,
            PresetCommands::Overlay { command } => match command {
                OverlayCommands::Link(cmd) => Box::pin(handle_overlay_link(&config, cmd)).await,
                OverlayCommands::Unlink => Box::pin(handle_overlay_unlink(&config)).await,
                OverlayCommands::Info => handle_overlay_info(&config),
            },
            PresetCommands::Pull => git_pull::handle_pull(&config, false).await,
        },
        Commands::List => Box::pin(list::handle_list(&config)).await,
        Commands::Info {
            target,
            diff,
            verbose,
        } => {
            if let Some(item) = system_info_item(&target, diff, verbose)? {
                Box::pin(sys::handle_info(&config, item)).await
            } else {
                Box::pin(info::handle_info(&config, &target, diff, verbose)).await
            }
        }
        Commands::Self_ { command } => match command {
            SelfCommands::Install { dest } => handle_self_install(config.clone(), dest).await,
            SelfCommands::Upgrade { channel } => handle_self_upgrade(&config, channel).await,
        },
        Commands::Serve { command } => match command {
            ServeCommands::Install(cmd) => serve::handle_install(&config, cmd.port).await,
            ServeCommands::Start(cmd) => serve::handle_start(&config, cmd.port).await,
            ServeCommands::Status => serve::handle_status(&config).await,
            ServeCommands::Uninstall => serve::handle_uninstall(&config).await,
            ServeCommands::Url(cmd) => serve::handle_url(&cmd.path, cmd.port),
        },
        Commands::Shell { command } => match command {
            ShellCommands::Init { force } => shells::handle_init_template(force).await,
            ShellCommands::List => Box::pin(shells::handle_list(&config)).await,
            ShellCommands::Info { target } => Box::pin(shells::handle_info(&config, &target)).await,
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
            EnvCommands::List { reveal } => env::commands::handle_list(&config, reveal).await,
            EnvCommands::Set { key, value, force } => {
                env::commands::handle_set(&config, &key, &value, force).await
            }
            EnvCommands::Delete { key, force } => {
                env::commands::handle_delete(&config, &key, force).await
            }
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
                    cmd.force,
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
                    cmd.no_workspace,
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
                EnvIdentitySubcommand::List => env::identity::handle_identity_list(&config).await,
            },
        },
        Commands::Sys { command } => match command {
            SysCommands::List { all } => Box::pin(sys::handle_list(&config, all)).await,
            SysCommands::Info { item } => Box::pin(sys::handle_info(&config, &item)).await,
            SysCommands::Status => Box::pin(sys::handle_status(&config)).await,
            SysCommands::Update {
                item,
                verbose,
                proxy,
            } => Box::pin(sys::handle_update(&config, item.as_deref(), verbose, proxy)).await,
            SysCommands::Init {
                preset,
                dry_run,
                force_profile,
                proxy,
            } => {
                Box::pin(sys::handle_init(
                    &config,
                    preset.as_deref(),
                    dry_run,
                    force_profile,
                    proxy,
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
        Commands::Ssh {
            remote_shell,
            with,
            with_secret,
            args,
        } => ssh::handle_ssh(&config, remote_shell, &with, &with_secret, &args).await,
        Commands::Local { command } => match command {
            LocalCommands::Download(cmd) => {
                ssh::handle_local_download(
                    &cmd.source,
                    cmd.destination.as_deref(),
                    cmd.force,
                    cmd.dry_run,
                    cmd.scp,
                )
                .await
            }
            LocalCommands::Upload(cmd) => {
                ssh::handle_local_upload(
                    &cmd.source,
                    cmd.destination.as_deref(),
                    cmd.force,
                    cmd.dry_run,
                    cmd.scp,
                )
                .await
            }
            LocalCommands::Status => ssh::handle_local_status().await,
        },
        Commands::Task { command } => match command {
            TaskCommands::Save {
                name,
                force,
                cwd,
                command,
            } => task::handle_save(&config, &name, force, cwd.as_deref(), command).await,
            TaskCommands::Run(cmd) => task::handle_run(&config, &cmd.name, &cmd.extra).await,
            TaskCommands::List => task::handle_list(&config).await,
            TaskCommands::Info { name } => task::handle_info(&config, &name).await,
            TaskCommands::Delete { name } => task::handle_delete(&config, &name).await,
        },
        // Top-level alias for `shine task run`; no independent semantics.
        Commands::Run(cmd) => task::handle_run(&config, &cmd.name, &cmd.extra).await,
    }
}

fn system_info_item(target: &str, diff: bool, verbose: bool) -> Result<Option<&str>> {
    let Some(item) = target.strip_prefix("sys/") else {
        return Ok(None);
    };
    if item.is_empty() {
        bail!("system info target must not be empty; use `shine info sys/<ITEM>`");
    }
    if diff || verbose {
        bail!("`--diff` and `--verbose` apply only to installed app and shell targets");
    }
    Ok(Some(item))
}

async fn warn_if_runtime_schema_pending(command: &Commands) {
    if !should_warn_runtime_schema(command) {
        return;
    }

    if let Ok(schema_version) = Config::read_global_runtime_schema_version().await
        && let Some(warning) = state::pending_schema_warning(schema_version)
    {
        eprintln!("{}", colors::yellow_stderr(&warning));
    }
}

fn should_warn_runtime_schema(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Init(_) | Commands::Completions { .. } | Commands::State { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_ssh_env_forwarding_before_destination() {
        let cli = Cli::try_parse_from([
            "shine",
            "ssh",
            "--with",
            "API_URL",
            "--with",
            "LOCAL_NAME=REMOTE_NAME",
            "--with-secret",
            "API_TOKEN",
            "-p",
            "2222",
            "dev",
            "printenv",
            "REMOTE_NAME",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Ssh {
                remote_shell,
                with,
                with_secret,
                args,
            } if remote_shell == RemoteShell::Posix
                && with == ["API_URL", "LOCAL_NAME=REMOTE_NAME"]
                && with_secret == ["API_TOKEN"]
                && args == ["-p", "2222", "dev", "printenv", "REMOTE_NAME"]
        ));
    }

    #[test]
    fn cli_parses_windows_remote_shell_before_destination() {
        let cli = Cli::try_parse_from([
            "shine",
            "ssh",
            "--remote-shell",
            "windows",
            "--with-secret",
            "GH_TOKEN=TOKEN",
            "intel.mac.local",
            "cmd",
            "/c",
            "echo",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Ssh {
                remote_shell: RemoteShell::Windows,
                with_secret,
                args,
                ..
            } if with_secret == ["GH_TOKEN=TOKEN"]
                && args == ["intel.mac.local", "cmd", "/c", "echo"]
        ));
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
                target: None,
                pull: false,
                diff: false,
                verbose: false,
                refresh_release: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: None,
                pull: false,
                diff: false,
                verbose: true,
                refresh_release: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--refresh-release"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: None,
                pull: false,
                diff: false,
                verbose: false,
                refresh_release: true
            })
        ));

        let cli = Cli::try_parse_from(["shine", "preset", "pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Pull
            }
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: None,
                pull: true,
                diff: false,
                verbose: false,
                refresh_release: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "update", "proxy/setproxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: Some(ref target),
                pull: false,
                diff: false,
                verbose: false,
                refresh_release: false
            }) if target == "proxy/setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "update", "--diff"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: None,
                pull: false,
                diff: true,
                verbose: false,
                refresh_release: false
            })
        ));

        let cli =
            Cli::try_parse_from(["shine", "update", "proxy/setproxy", "--pull", "--diff"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: Some(ref target),
                pull: true,
                diff: true,
                verbose: false,
                refresh_release: false
            }) if target == "proxy/setproxy"
        ));

        assert!(Cli::try_parse_from(["shine", "update", "proxy/setproxy", "--verbose"]).is_err());
        assert!(
            Cli::try_parse_from(["shine", "update", "proxy/setproxy", "--refresh-release"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["shine", "update", "--refresh"]).is_err());

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

        let cli = Cli::try_parse_from(["shine", "state", "migrate"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::State {
                command: StateCommands::Migrate(StateMigrateCommand { dry_run: false })
            }
        ));

        let cli = Cli::try_parse_from(["shine", "state", "migrate", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::State {
                command: StateCommands::Migrate(StateMigrateCommand { dry_run: true })
            }
        ));
        assert!(Cli::try_parse_from(["shine", "clear"]).is_err());
    }

    #[test]
    fn cli_accepts_preset_commands() {
        let cli = Cli::try_parse_from(["shine", "preset", "export"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Export(commands::ExportCommand {
                    dir: None,
                    force: false
                })
            }
        ));

        let cli = Cli::try_parse_from(["shine", "preset", "copy", "app/surge"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Copy(commands::CopyCommand {
                    target,
                    force: false
                })
            } if target == "app/surge"
        ));

        let cli =
            Cli::try_parse_from(["shine", "preset", "copy", "shell/proxy", "--force"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Copy(commands::CopyCommand {
                    target,
                    force: true
                })
            } if target == "shell/proxy"
        ));
        assert!(Cli::try_parse_from(["shine", "preset", "copy"]).is_err());
        for invalid in [
            "surge",
            "app/",
            "/app/surge",
            "app/../surge",
            "app/surge/extra",
            "other/surge",
        ] {
            assert!(
                Cli::try_parse_from(["shine", "preset", "copy", invalid]).is_err(),
                "target should be rejected during CLI parsing: {invalid}"
            );
        }

        let cli =
            Cli::try_parse_from(["shine", "preset", "link", "/tmp/presets", "--create"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Link(commands::LinkCommand { create: true, .. })
            }
        ));

        let cli = Cli::try_parse_from(["shine", "preset", "unlink"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Unlink
            }
        ));

        for legacy in ["export", "link", "unlink", "overlay", "pull"] {
            assert!(Cli::try_parse_from(["shine", legacy]).is_err());
        }
    }

    #[test]
    fn cli_accepts_overlay_commands() {
        let cli =
            Cli::try_parse_from(["shine", "preset", "overlay", "link", "/tmp/presets"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Overlay {
                    command: OverlayCommands::Link(OverlayLinkCommand {
                        ref path,
                        git: None,
                        create: false,
                        ..
                    })
                }
            } if path.as_deref() == Some(std::path::Path::new("/tmp/presets"))
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "preset",
            "overlay",
            "link",
            "/tmp/presets",
            "--create",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Overlay {
                    command: OverlayCommands::Link(OverlayLinkCommand {
                        ref path,
                        create: true,
                        ..
                    })
                }
            } if path.as_deref() == Some(std::path::Path::new("/tmp/presets"))
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "preset",
            "overlay",
            "link",
            "--git",
            "https://example.com/overlay.git",
            "--branch",
            "main",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Overlay {
                    command: OverlayCommands::Link(OverlayLinkCommand {
                        path: None,
                        git: Some(ref url),
                        branch: Some(ref branch),
                        ..
                    })
                }
            } if url == "https://example.com/overlay.git" && branch == "main"
        ));

        // A local PATH and --git are mutually exclusive.
        assert!(
            Cli::try_parse_from(["shine", "preset", "overlay", "link", "/tmp/x", "--git", "u"])
                .is_err()
        );

        assert!(Cli::try_parse_from(["shine", "preset", "overlay", "unlink"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "preset", "overlay", "info"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "preset", "overlay", "show"]).is_err());
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
    fn cli_accepts_env_list_reveal() {
        let cli = Cli::try_parse_from(["shine", "env", "list", "--reveal"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::List { reveal: true }
            }
        ));
        assert!(Cli::try_parse_from(["shine", "env", "show"]).is_err());
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
                command: EnvCommands::Delete { key, .. }
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
    fn cli_accepts_env_run_no_workspace_with_explicit_values() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "run",
            "--no-workspace",
            "--with",
            "API_URL",
            "--",
            "bun",
            "tool.ts",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Run(cmd)
            } if cmd.no_workspace
                && cmd.with == ["API_URL"]
                && cmd.command == ["bun", "tool.ts"]
        ));
    }

    #[test]
    fn cli_rejects_env_run_no_workspace_with_mode() {
        let error = Cli::try_parse_from([
            "shine",
            "env",
            "run",
            "--no-workspace",
            "--mode",
            "production",
            "--",
            "bun",
            "tool.ts",
        ])
        .unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "--no-workspace must conflict with --mode"
        );
    }

    #[test]
    fn cli_accepts_task_save_with_trailing_command() {
        let cli = Cli::try_parse_from([
            "shine",
            "task",
            "save",
            "port-3000",
            "--",
            "lsof",
            "-i",
            ":3000",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Task {
                command: TaskCommands::Save {
                    name,
                    force: false,
                    cwd: None,
                    command,
                }
            } if name == "port-3000" && command == ["lsof", "-i", ":3000"]
        ));
    }

    #[test]
    fn cli_accepts_task_save_force_flag() {
        let cli = Cli::try_parse_from([
            "shine", "task", "save", "deploy", "--force", "--", "rsync", "-avz", "dist/",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Task {
                command: TaskCommands::Save {
                    name,
                    force: true,
                    cwd: None,
                    command,
                }
            } if name == "deploy" && command == ["rsync", "-avz", "dist/"]
        ));
    }

    #[test]
    fn cli_accepts_task_save_with_cwd() {
        let cli = Cli::try_parse_from([
            "shine", "task", "save", "build", "--cwd", ".", "--", "cargo", "build",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Task {
                command: TaskCommands::Save {
                    name,
                    force: false,
                    cwd: Some(cwd),
                    command,
                }
            } if name == "build" && cwd == std::path::Path::new(".") && command == ["cargo", "build"]
        ));
    }

    #[test]
    fn cli_accepts_task_run_with_extra_args() {
        let cli =
            Cli::try_parse_from(["shine", "task", "run", "lsof-port", "--", ":3000"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Task {
                command: TaskCommands::Run(ref cmd)
            } if cmd.name == "lsof-port" && cmd.extra == [":3000"]
        ));
    }

    #[test]
    fn cli_accepts_top_level_run_alias_with_hyphen_args() {
        let cli = Cli::try_parse_from(["shine", "run", "build", "--", "--flag"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Run(ref cmd) if cmd.name == "build" && cmd.extra == ["--flag"]
        ));

        let cli = Cli::try_parse_from(["shine", "run", "build"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Run(ref cmd) if cmd.name == "build" && cmd.extra.is_empty()
        ));
    }

    #[test]
    fn cli_accepts_task_list_info_delete() {
        assert!(matches!(
            Cli::try_parse_from(["shine", "task", "list"])
                .unwrap()
                .command,
            Commands::Task {
                command: TaskCommands::List
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["shine", "task", "info", "deploy"]).unwrap().command,
            Commands::Task {
                command: TaskCommands::Info { name }
            } if name == "deploy"
        ));
        assert!(matches!(
            Cli::try_parse_from(["shine", "task", "delete", "deploy"]).unwrap().command,
            Commands::Task {
                command: TaskCommands::Delete { name }
            } if name == "deploy"
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
        let state = Commands::State {
            command: StateCommands::Migrate(StateMigrateCommand { dry_run: false }),
        };
        let list = Commands::List;

        assert!(!should_warn_runtime_schema(&init));
        assert!(!should_warn_runtime_schema(&completions));
        assert!(!should_warn_runtime_schema(&state));
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
    fn top_level_info_routes_only_explicit_system_targets() {
        assert_eq!(
            system_info_item("sys/split-dns", false, false).unwrap(),
            Some("split-dns")
        );
        assert_eq!(system_info_item("split-dns", false, false).unwrap(), None);
        assert!(system_info_item("sys/", false, false).is_err());
        assert!(system_info_item("sys/split-dns", true, false).is_err());
        assert!(system_info_item("sys/split-dns", false, true).is_err());
    }

    #[test]
    fn cli_accepts_shell_info_and_identity_list() {
        let cli = Cli::try_parse_from(["shine", "shell", "info", "proxy/setproxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Info { target }
            } if target == "proxy/setproxy"
        ));

        assert!(Cli::try_parse_from(["shine", "env", "identity", "list"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "env", "identity", "show"]).is_err());
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
    fn cli_accepts_app_build_command() {
        let cli = Cli::try_parse_from(["shine", "app", "build", "surge"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Build { app_id }
            } if app_id == "surge"
        ));
    }

    #[test]
    fn cli_accepts_app_unbuild_command() {
        let cli = Cli::try_parse_from(["shine", "app", "unbuild", "surge"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Unbuild { app_id }
            } if app_id == "surge"
        ));
    }

    #[test]
    fn cli_accepts_app_refresh_commands() {
        let cli = Cli::try_parse_from(["shine", "app", "refresh", "surge"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Refresh {
                    category,
                    file: None,
                    force: false,
                }
            } if category == "surge"
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "app",
            "refresh",
            "surge",
            "subscription-proxies.conf",
            "--force",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Refresh {
                    category,
                    file: Some(file),
                    force: true,
                }
            } if category == "surge" && file == "subscription-proxies.conf"
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
                    force_profile: false,
                    proxy: false
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
                    force_profile: false,
                    proxy: false
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
                    force_profile: false,
                    proxy: false
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
                    force_profile: true,
                    proxy: false
                }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "init", "--proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Init {
                    preset: None,
                    dry_run: false,
                    force_profile: false,
                    proxy: true
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
                    force_profile: false,
                    proxy: false
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
    fn cli_accepts_sys_update_options() {
        let cli = Cli::try_parse_from(["shine", "sys", "update"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Update {
                    item: None,
                    verbose: false,
                    proxy: false
                }
            }
        ));
        let cli = Cli::try_parse_from(["shine", "sys", "update", "neovim", "--verbose", "--proxy"])
            .unwrap();
        assert!(
            matches!(cli.command, Commands::Sys { command: SysCommands::Update { item: Some(ref item), verbose: true, proxy: true } } if item == "neovim")
        );
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
    fn cli_accepts_unified_serve_commands() {
        let cli = Cli::try_parse_from(["shine", "serve", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Serve {
                command: ServeCommands::Install(_)
            }
        ));

        let cli = Cli::try_parse_from(["shine", "serve", "start"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Serve {
                command: ServeCommands::Start(_)
            }
        ));

        let cli = Cli::try_parse_from(["shine", "serve", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Serve {
                command: ServeCommands::Status
            }
        ));

        let cli = Cli::try_parse_from(["shine", "serve", "uninstall"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Serve {
                command: ServeCommands::Uninstall
            }
        ));

        let cli = Cli::try_parse_from(["shine", "serve", "url", "app/surge/custom-rules.sgmodule"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Serve {
                command: ServeCommands::Url(_)
            }
        ));
    }

    #[test]
    fn cli_rejects_removed_presets_subcommands() {
        assert!(Cli::try_parse_from(["shine", "presets"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "export"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "link", "/tmp/presets"]).is_err());
        assert!(Cli::try_parse_from(["shine", "presets", "unlink"]).is_err());
    }
}
