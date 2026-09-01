use anyhow::{Context, Result, bail};
use clap::Parser;

use cli::{
    apps, colors, commands, completion, config, env, git_pull, info, list, serve, shells, ssh,
    state, sys, task, theme, update_check,
};

use commands::{
    AppArtifactCommands, AppCommands, Cli, Commands, CompletionCommands, CompletionShell,
    EnvBrokerPolicySubcommand, EnvBrokerSubcommand, EnvCommands, EnvIdentitySubcommand,
    EnvProxySubcommand, EnvSecretSubcommand, EnvWorkspaceSubcommand, LocalCommands,
    OverlayCommands, PresetCommands, PresetTemplateKind, ResourceKind, SelfCommands, ServeCommands,
    ShellCommands, StateCommands, SysCommands, SysProfileCommands, TaskCommands, ThemeCommands,
    TrustCommands,
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
use cli::shim::{handle_install_shim_approved, handle_uninstall_shim_approved};

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
        // State migration must be able to read retired config fields before
        // normal config loading rejects them, and applies its own writes.
        let config = Box::pin(Config::load_global_runtime_for_dry_run()).await?;
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

    if let Commands::Preset {
        command: PresetCommands::Validate { path, format },
    } = &cli.command
    {
        let valid = cli::preset_validation::handle_validate(path, *format).await?;
        if !valid {
            std::process::exit(1);
        }
        return Ok(());
    }

    let config = Box::pin(Config::load_or_init()).await?;

    if config.legacy_allow_app_hooks || config.legacy_allow_sys_code {
        eprintln!(
            "Warning: allow_app_hooks/allow_sys_code are retired and ignored; review current external code with `shine trust inspect <TARGET>` and enroll it with `shine trust grant <TARGET>`."
        );
    }

    if let Commands::ShellRender { target } = &cli.command {
        return shells::handle_render_live(&config, target).await;
    }

    warn_if_runtime_schema_pending(&cli.command).await;

    update_check::maybe_notify(&config, &cli.command).await?;

    match cli.command {
        Commands::ShellRender { .. } => unreachable!(),
        Commands::Init(_) => unreachable!(),
        Commands::Completions {
            command: CompletionCommands::Install,
        } => Box::pin(shells::handle_completion_install(&config)).await,
        Commands::Completions { .. } => unreachable!(),
        Commands::State { .. } => unreachable!(),
        Commands::Theme { .. } => unreachable!(),
        Commands::Trust { command } => match command {
            TrustCommands::List => cli::trust::handle_list(&config).await,
            TrustCommands::Inspect { target } => cli::trust::handle_inspect(&config, &target).await,
            TrustCommands::Grant { target, yes } => {
                cli::trust::handle_grant(&config, &target, yes).await
            }
            TrustCommands::Revoke { target } => cli::trust::handle_revoke(&config, &target).await,
        },
        Commands::Install {
            target,
            replace_managed,
            yes,
        } => handle_install_shim_approved(&config, &target, replace_managed, yes).await,
        Commands::Uninstall {
            target,
            force,
            purge,
            dry_run,
            yes,
        } => handle_uninstall_shim_approved(&config, &target, force, purge, dry_run, yes).await,
        Commands::App { command } => match command {
            AppCommands::List => Box::pin(apps::handle_list(&config)).await,
            AppCommands::Info {
                category,
                run_generators,
                diff,
            } => Box::pin(apps::handle_info(&config, &category, run_generators, diff)).await,
            AppCommands::Install {
                category,
                dry_run,
                replace_managed,
                yes,
            } => {
                Box::pin(apps::handle_install_approved(
                    &config,
                    category.as_deref(),
                    dry_run,
                    replace_managed,
                    yes,
                ))
                .await
            }
            AppCommands::Refresh {
                category,
                file,
                force,
                yes,
            } => {
                Box::pin(apps::handle_refresh_approved(
                    &config,
                    &category,
                    file.as_deref(),
                    force,
                    yes,
                ))
                .await
            }
            AppCommands::Recover { yes } => {
                Box::pin(apps::handle_recover_approved(&config, yes)).await
            }
            AppCommands::Uninstall {
                category,
                force,
                purge,
                dry_run,
                yes,
            } => {
                Box::pin(apps::handle_uninstall_approved(
                    &config,
                    category.as_deref(),
                    force,
                    purge,
                    dry_run,
                    yes,
                ))
                .await
            }
            AppCommands::Artifact { command } => match command {
                AppArtifactCommands::Apply { app_id, yes } => {
                    Box::pin(apps::handle_build_approved(&config, &app_id, yes)).await
                }
                AppArtifactCommands::Remove { app_id, yes } => {
                    Box::pin(apps::handle_unbuild_approved(&config, &app_id, yes)).await
                }
            },
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
                    cmd.run_generators,
                )
                .await
            } else {
                handle_update(
                    &config,
                    cmd.target.as_deref(),
                    cmd.diff,
                    cmd.verbose,
                    cmd.refresh_release,
                    cmd.run_generators,
                )
                .await
            }
        }
        Commands::Upgrade(cmd) => {
            if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                let config = Box::pin(Config::load_or_init()).await?;
                handle_config_upgrade(
                    &config,
                    cmd.target.as_deref(),
                    cmd.verbose,
                    cmd.prune_stale,
                    cmd.yes,
                )
                .await
            } else {
                handle_config_upgrade(
                    &config,
                    cmd.target.as_deref(),
                    cmd.verbose,
                    cmd.prune_stale,
                    cmd.yes,
                )
                .await
            }
        }
        Commands::Preset { command } => match command {
            PresetCommands::New { kind, force } => match kind {
                PresetTemplateKind::App => apps::handle_init_template(force).await,
                PresetTemplateKind::Shell => shells::handle_init_template(force).await,
                PresetTemplateKind::Sys => sys::handle_init_template(force).await,
            },
            PresetCommands::Validate { .. } => unreachable!(),
            PresetCommands::Export(cmd) => {
                Box::pin(handle_preset_export(&config, cmd.dir, cmd.force)).await
            }
            PresetCommands::Copy(cmd) => Box::pin(handle_preset_copy(&cmd.target, cmd.force)).await,
            PresetCommands::Link(cmd) => {
                Box::pin(handle_preset_link(&config, cmd.path, cmd.create, cmd.live)).await
            }
            PresetCommands::Unlink => Box::pin(handle_preset_unlink(&config)).await,
            PresetCommands::Overlay { command } => match command {
                OverlayCommands::Link(cmd) => Box::pin(handle_overlay_link(&config, cmd)).await,
                OverlayCommands::Unlink => Box::pin(handle_overlay_unlink(&config)).await,
                OverlayCommands::Info => handle_overlay_info(&config),
            },
            PresetCommands::Pull => git_pull::handle_pull(&config, false).await,
        },
        Commands::List { available, kind } => {
            if available {
                handle_available_list(&config, kind).await
            } else {
                Box::pin(list::handle_list(&config)).await
            }
        }
        Commands::Info {
            target,
            diff,
            verbose,
            run_generators,
        } => {
            if let Some(item) = system_info_item(&target, diff, verbose)? {
                if run_generators {
                    anyhow::bail!("--run-generators requires an App target");
                }
                Box::pin(sys::handle_info(&config, item)).await
            } else {
                Box::pin(info::handle_info(
                    &config,
                    &target,
                    diff,
                    verbose,
                    run_generators,
                ))
                .await
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
            ShellCommands::List => Box::pin(shells::handle_list(&config)).await,
            ShellCommands::Info { target } => Box::pin(shells::handle_info(&config, &target)).await,
            ShellCommands::Recover { yes } => {
                Box::pin(shells::handle_recover_approved(&config, yes)).await
            }
            ShellCommands::Install {
                target,
                dry_run: true,
                replace_managed: _,
                yes: _,
            } => Box::pin(shells::handle_install_dry_run(&config, target.as_deref())).await,
            ShellCommands::Install {
                target,
                dry_run: false,
                replace_managed,
                yes,
            } => {
                Box::pin(shells::handle_install_approved(
                    &config,
                    target.as_deref(),
                    replace_managed,
                    yes,
                ))
                .await
            }
            ShellCommands::Uninstall {
                target,
                purge,
                dry_run,
                yes,
            } => {
                Box::pin(shells::handle_uninstall_approved(
                    &config,
                    target.as_deref(),
                    purge,
                    dry_run,
                    yes,
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
            EnvCommands::Run(cmd) => {
                env::workspace::handle_run(
                    &config,
                    cmd.workspace.as_deref(),
                    cmd.mode.as_deref(),
                    cmd.no_workspace,
                    &cmd.with,
                    cmd.secret_broker,
                    &cmd.secret,
                    &cmd.command,
                )
                .await
            }
            EnvCommands::Workspace(cmd) => match cmd.command {
                EnvWorkspaceSubcommand::Init(cmd) => {
                    env::workspace::handle_init_from_dotenv(
                        cmd.from_dotenv,
                        &cmd.mode,
                        &cmd.secret,
                        cmd.force,
                        cmd.dry_run,
                    )
                    .await
                }
                EnvWorkspaceSubcommand::Export(cmd) => {
                    env::workspace::handle_export(
                        &config,
                        cmd.format,
                        cmd.workspace.as_deref(),
                        &cmd.mode,
                        &cmd.output,
                        cmd.include_secrets,
                        cmd.force,
                        cmd.dry_run,
                    )
                    .await
                }
            },
            EnvCommands::Broker(cmd) => match cmd.command {
                EnvBrokerSubcommand::Describe {
                    workspace,
                    mode,
                    release,
                    release_all_declared,
                    command,
                } => {
                    env::broker::handle_describe(
                        workspace.as_deref(),
                        &mode,
                        &release,
                        release_all_declared,
                        &command,
                    )
                    .await
                }
                EnvBrokerSubcommand::Policy(cmd) => match cmd.command {
                    EnvBrokerPolicySubcommand::Add(input) => {
                        env::broker::handle_policy_add(
                            &config,
                            &input.name,
                            &input.ssh_target,
                            &input.project,
                            &input.workspace,
                            input.remote_workspace.as_deref(),
                            &input.mode,
                            &input.release,
                            input.release_all_declared,
                            &input.command,
                        )
                        .await
                    }
                    EnvBrokerPolicySubcommand::Update(input) => {
                        env::broker::handle_policy_update(
                            &config,
                            &input.name,
                            &input.ssh_target,
                            &input.project,
                            &input.workspace,
                            input.remote_workspace.as_deref(),
                            &input.mode,
                            &input.release,
                            input.release_all_declared,
                            &input.command,
                        )
                        .await
                    }
                    EnvBrokerPolicySubcommand::Diff {
                        name,
                        workspace,
                        mode,
                        release,
                        release_all_declared,
                        command,
                    } => {
                        env::broker::handle_policy_diff(
                            &config,
                            &name,
                            &workspace,
                            &mode,
                            &release,
                            release_all_declared,
                            &command,
                        )
                        .await
                    }
                    EnvBrokerPolicySubcommand::List => {
                        env::broker::handle_policy_list(&config).await
                    }
                    EnvBrokerPolicySubcommand::Info { name } => {
                        env::broker::handle_policy_info(&config, &name).await
                    }
                    EnvBrokerPolicySubcommand::Remove { name } => {
                        env::broker::handle_policy_remove(&config, &name).await
                    }
                },
            },
            EnvCommands::Proxy(cmd) => match cmd.command {
                EnvProxySubcommand::Install {
                    command,
                    with,
                    project,
                } => env::proxy::install(&config, &command, &with, project).await,
                EnvProxySubcommand::List => env::proxy::list(&config).await,
                EnvProxySubcommand::Uninstall { command } => {
                    env::proxy::uninstall(&config, &command).await
                }
                EnvProxySubcommand::Enable { command, project } => {
                    env::proxy::set_enabled(&config, &command, true, project).await
                }
                EnvProxySubcommand::Disable { command, project } => {
                    env::proxy::set_enabled(&config, &command, false, project).await
                }
                EnvProxySubcommand::Exec {
                    target,
                    command,
                    args,
                } => env::proxy::exec(&config, &target, &command, &args).await,
            },
            EnvCommands::Secret(cmd) => match cmd.command {
                EnvSecretSubcommand::Decrypt { key } => {
                    env::commands::handle_decrypt(&config, &key).await
                }
                EnvSecretSubcommand::Export { key, alias } => {
                    env::commands::handle_export(&config, &key, alias.as_deref()).await
                }
                EnvSecretSubcommand::Encrypt(cmd) => {
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
                EnvSecretSubcommand::Seal(cmd) => {
                    env::workspace::handle_seal(
                        &config,
                        cmd.workspace.as_deref(),
                        cmd.file.as_deref(),
                        cmd.backend.as_deref(),
                        &cmd.recipients,
                    )
                    .await
                }
                EnvSecretSubcommand::Identity(cmd) => match cmd.command {
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
                    EnvIdentitySubcommand::List => {
                        env::identity::handle_identity_list(&config).await
                    }
                },
            },
        },
        Commands::Sys { command } => match command {
            SysCommands::Recover { yes } => {
                Box::pin(sys::handle_recover_approved(&config, yes)).await
            }
            SysCommands::List { all } => Box::pin(sys::handle_list(&config, all)).await,
            SysCommands::Info { item } => Box::pin(sys::handle_info(&config, &item)).await,
            SysCommands::Status => Box::pin(sys::handle_status(&config)).await,
            SysCommands::Bootstrap {
                items,
                exact_items,
                preset,
                dry_run,
                force_profile,
                proxy,
                yes,
            } => {
                let requested = if exact_items.is_empty() {
                    items
                } else {
                    exact_items
                };
                Box::pin(sys::handle_init(
                    &config,
                    &requested,
                    preset.as_deref(),
                    dry_run,
                    force_profile,
                    proxy,
                    yes,
                ))
                .await
            }
            SysCommands::Profile { command } => match command {
                SysProfileCommands::Enable { item, dry_run, yes } => {
                    Box::pin(sys::handle_profile_enable_approved(
                        &config, &item, dry_run, yes,
                    ))
                    .await
                }
                SysProfileCommands::Disable { item, dry_run, yes } => {
                    Box::pin(sys::handle_profile_disable_approved(
                        &config, &item, dry_run, yes,
                    ))
                    .await
                }
            },
            SysCommands::Apply { item, dry_run, yes } => {
                Box::pin(sys::handle_apply_approved(
                    &config,
                    item.as_deref(),
                    dry_run,
                    yes,
                ))
                .await
            }
            SysCommands::Uninstall { item, dry_run, yes } => {
                Box::pin(sys::handle_uninstall_approved(&config, &item, dry_run, yes)).await
            }
        },
        Commands::Ssh {
            remote_shell,
            with,
            with_secret,
            secret_broker,
            secret_broker_policy,
            allow_secret,
            trust_remote_session,
            secret_broker_inspect,
            secret_broker_enroll,
            trust_remote_metadata,
            secret_broker_update_policy,
            args,
        } => {
            ssh::handle_ssh(
                &config,
                remote_shell,
                &with,
                &with_secret,
                secret_broker,
                &secret_broker_policy,
                &allow_secret,
                trust_remote_session,
                secret_broker_inspect,
                secret_broker_enroll,
                trust_remote_metadata,
                secret_broker_update_policy.as_deref(),
                &args,
            )
            .await
        }
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

async fn handle_available_list(config: &Config, kind: Option<ResourceKind>) -> Result<()> {
    match kind {
        Some(ResourceKind::App) => Box::pin(apps::handle_list(config)).await,
        Some(ResourceKind::Shell) => Box::pin(shells::handle_list(config)).await,
        Some(ResourceKind::Sys) => Box::pin(sys::handle_list(config, false)).await,
        None => {
            config::print_presets_note(config);
            Box::pin(shells::handle_list_with_presets_note(config, false)).await?;
            println!();
            Box::pin(apps::handle_list_with_presets_note(config, false)).await?;
            println!();
            Box::pin(sys::handle_list(config, false)).await
        }
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
                ..
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
    fn cli_parses_direct_ssh_secret_broker() {
        let cli = Cli::try_parse_from([
            "shine",
            "ssh",
            "--secret-broker",
            "--secret-broker-policy",
            "/tmp/team-policy.toml",
            "--allow-secret",
            "API_TOKEN",
            "--allow-secret",
            "NPM_TOKEN=NODE_AUTH_TOKEN",
            "dev",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Ssh {
                secret_broker: true,
                secret_broker_policy,
                allow_secret,
                trust_remote_session: false,
                args,
                ..
            } if secret_broker_policy == [std::path::PathBuf::from("/tmp/team-policy.toml")]
                && allow_secret == ["API_TOKEN", "NPM_TOKEN=NODE_AUTH_TOKEN"]
                && args == ["dev"]
        ));
    }

    #[test]
    fn cli_parses_remote_direct_broker_run() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "run",
            "--no-workspace",
            "--secret-broker",
            "--secret",
            "API_TOKEN",
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
            } if cmd.no_workspace
                && cmd.secret_broker
                && cmd.secret == ["API_TOKEN"]
                && cmd.command == ["bun", "run", "build"]
        ));
    }

    #[test]
    fn cli_parses_broker_policy_add() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "broker",
            "policy",
            "add",
            "--name",
            "dev-api",
            "--ssh-target",
            "dev",
            "--workspace",
            "/src/api/shine.workspace.toml",
            "--mode",
            "development",
            "--release",
            "API_TOKEN",
            "--",
            "bun",
            "run",
            "build",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Broker(_)
            }
        ));
    }

    #[test]
    fn cli_parses_all_declared_broker_release() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "broker",
            "describe",
            "--mode",
            "production",
            "--release-all-declared",
            "--",
            "bun",
            "start",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Broker(cmd)
            } if matches!(
                &cmd.command,
                EnvBrokerSubcommand::Describe {
                    release,
                    release_all_declared: true,
                    command,
                    ..
                } if release.is_empty() && command.as_slice() == ["bun", "start"]
            )
        ));
    }

    #[test]
    fn cli_rejects_mixed_or_missing_broker_release_selection() {
        assert!(
            Cli::try_parse_from([
                "shine",
                "env",
                "broker",
                "describe",
                "--mode",
                "production",
                "--release",
                "TOKEN",
                "--release-all-declared",
                "--",
                "bun",
                "start",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "shine",
                "env",
                "broker",
                "describe",
                "--mode",
                "production",
                "--",
                "bun",
                "start",
            ])
            .is_err()
        );
    }

    #[test]
    fn cli_parses_trusted_remote_policy_update() {
        let cli = Cli::try_parse_from([
            "shine",
            "ssh",
            "--secret-broker-enroll",
            "--trust-remote-metadata",
            "--update-policy",
            "intel-shine-bot-production",
            "intel.mac.local",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Ssh {
                secret_broker_enroll: true,
                trust_remote_metadata: true,
                secret_broker_update_policy: Some(name),
                args,
                ..
            } if name == "intel-shine-bot-production" && args == ["intel.mac.local"]
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
                refresh_release: false,
                run_generators: false
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
                refresh_release: false,
                run_generators: false
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
                refresh_release: true,
                run_generators: false
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
                refresh_release: false,
                run_generators: false
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
                refresh_release: false,
                run_generators: false
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
                refresh_release: false,
                run_generators: false
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
                refresh_release: false,
                run_generators: false
            }) if target == "proxy/setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "update", "utils/shine-theme-sync", "--verbose"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                target: Some(ref target),
                pull: false,
                diff: false,
                verbose: true,
                refresh_release: false,
                run_generators: false
            }) if target == "utils/shine-theme-sync"
        ));
        assert!(
            Cli::try_parse_from(["shine", "update", "proxy/setproxy", "--refresh-release"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["shine", "update", "--refresh"]).is_err());

        let cli = Cli::try_parse_from(["shine", "update", "--run-generators"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Update(UpdateCommand {
                run_generators: true,
                ..
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                target: None,
                pull: false,
                verbose: false,
                prune_stale: false,
                yes: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                target: None,
                pull: false,
                verbose: true,
                prune_stale: false,
                yes: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--prune-stale"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                target: None,
                pull: false,
                verbose: false,
                prune_stale: true,
                yes: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "--pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                target: None,
                pull: true,
                verbose: false,
                prune_stale: false,
                yes: false
            })
        ));

        let cli = Cli::try_parse_from(["shine", "upgrade", "app/starship"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Upgrade(UpgradeCommand {
                target: Some(ref target),
                pull: false,
                verbose: false,
                prune_stale: false,
                yes: false,
            }) if target == "app/starship"
        ));

        let cli = Cli::try_parse_from(["shine", "list", "--available", "app"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::List {
                available: true,
                kind: Some(ResourceKind::App),
            }
        ));
        assert!(Cli::try_parse_from(["shine", "list", "app"]).is_err());

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
    fn lifecycle_commands_accept_yes_and_reject_dry_run_conflicts() {
        for args in [
            vec!["shine", "install", "app/demo", "--yes"],
            vec!["shine", "uninstall", "app/demo", "--yes"],
            vec!["shine", "upgrade", "--yes"],
            vec!["shine", "app", "install", "demo", "--yes"],
            vec!["shine", "app", "refresh", "demo", "--yes"],
            vec!["shine", "app", "recover", "--yes"],
            vec!["shine", "app", "uninstall", "demo", "--yes"],
            vec!["shine", "app", "artifact", "apply", "demo", "--yes"],
            vec!["shine", "app", "artifact", "remove", "demo", "--yes"],
            vec!["shine", "shell", "install", "demo", "--yes"],
            vec!["shine", "shell", "uninstall", "demo", "--yes"],
            vec!["shine", "sys", "profile", "enable", "demo", "--yes"],
            vec!["shine", "sys", "profile", "disable", "demo", "--yes"],
            vec!["shine", "sys", "apply", "demo", "--yes"],
            vec!["shine", "sys", "uninstall", "demo", "--yes"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }

        for args in [
            vec!["shine", "uninstall", "app/demo", "--dry-run", "--yes"],
            vec!["shine", "app", "install", "demo", "--dry-run", "--yes"],
            vec!["shine", "app", "uninstall", "demo", "--dry-run", "--yes"],
            vec!["shine", "shell", "install", "demo", "--dry-run", "--yes"],
            vec!["shine", "shell", "uninstall", "demo", "--dry-run", "--yes"],
            vec![
                "shine",
                "sys",
                "profile",
                "enable",
                "demo",
                "--dry-run",
                "--yes",
            ],
            vec![
                "shine",
                "sys",
                "profile",
                "disable",
                "demo",
                "--dry-run",
                "--yes",
            ],
            vec!["shine", "sys", "apply", "demo", "--dry-run", "--yes"],
            vec!["shine", "sys", "uninstall", "demo", "--dry-run", "--yes"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
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

        let cli =
            Cli::try_parse_from(["shine", "preset", "link", "/tmp/presets", "--live"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Link(commands::LinkCommand { live: true, .. })
            }
        ));

        let cli = Cli::try_parse_from(["shine", "preset", "unlink"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Unlink
            }
        ));

        let cli = Cli::try_parse_from(["shine", "preset", "validate"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Validate {
                    path,
                    format: commands::PresetValidationFormat::Text,
                }
            } if path == std::path::Path::new(".")
        ));
        let cli = Cli::try_parse_from([
            "shine",
            "preset",
            "validate",
            "presets/app/git/shine.toml",
            "--format",
            "json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Validate {
                    path,
                    format: commands::PresetValidationFormat::Json,
                }
            } if path == std::path::Path::new("presets/app/git/shine.toml")
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
    fn cli_accepts_env_secret_export_command() {
        let cli =
            Cli::try_parse_from(["shine", "env", "secret", "export", "DEEPSEEK_API_KEY"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Export { key, alias: None }
                })
            } if key == "DEEPSEEK_API_KEY"
        ));
    }

    #[test]
    fn cli_accepts_primary_env_secret_commands() {
        assert!(matches!(
            Cli::try_parse_from(["shine", "env", "secret", "decrypt", "TOKEN_SECRET"])
                .unwrap()
                .command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Decrypt { key }
                })
            } if key == "TOKEN_SECRET"
        ));
        assert!(Cli::try_parse_from(["shine", "env", "secret", "identity", "list",]).is_ok());
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
    fn cli_accepts_env_secret_export_with_alias() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "secret",
            "export",
            "DEEPSEEK_API_KEY",
            "--as",
            "AI_KEY",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Export { key, alias: Some(a) }
                })
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
    fn cli_accepts_env_secret_encrypt_without_recipient() {
        let cli = Cli::try_parse_from(["shine", "env", "secret", "encrypt", "--from", "MY_TOKEN"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Encrypt(cmd)
                })
            } if cmd.recipients.is_empty() && cmd.from.as_deref() == Some("MY_TOKEN")
        ));
    }

    #[test]
    fn cli_accepts_env_secret_encrypt_with_recipient() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "secret",
            "encrypt",
            "-r",
            "alice@example.com",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Encrypt(cmd)
                })
            } if cmd.recipients == ["alice@example.com"]
        ));
    }

    #[test]
    fn cli_accepts_workspace_env_secret_seal() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "secret",
            "seal",
            ".env.production.shine.toml",
            "--recipient",
            "alice@example.com",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Secret(commands::EnvSecretCommand {
                    command: EnvSecretSubcommand::Seal(cmd)
                })
            } if cmd.file.as_deref() == Some(std::path::Path::new(".env.production.shine.toml"))
                && cmd.recipients == ["alice@example.com"]
        ));
    }

    #[test]
    fn cli_accepts_workspace_dotenv_init() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "workspace",
            "init",
            "--from-dotenv",
            "--mode",
            "development",
            "--secret",
            "DATABASE_URL",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Workspace(commands::EnvWorkspaceCommand {
                    command: EnvWorkspaceSubcommand::Init(cmd)
                })
            } if cmd.from_dotenv
                && cmd.mode == ["development"]
                && cmd.secret == ["DATABASE_URL"]
                && cmd.dry_run
        ));
    }

    #[test]
    fn cli_accepts_workspace_dotenv_export() {
        let cli = Cli::try_parse_from([
            "shine",
            "env",
            "workspace",
            "export",
            "--format",
            "dotenv",
            "--mode",
            "production",
            "--output",
            ".env.production.local",
            "--include-secrets",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                command: EnvCommands::Workspace(commands::EnvWorkspaceCommand {
                    command: EnvWorkspaceSubcommand::Export(cmd)
                })
            } if cmd.format == commands::EnvWorkspaceExportFormat::Dotenv
                && cmd.mode == "production"
                && cmd.output == std::path::Path::new(".env.production.local")
                && cmd.include_secrets
                && cmd.dry_run
        ));
    }

    #[test]
    fn cli_requires_workspace_export_format() {
        let error = Cli::try_parse_from([
            "shine",
            "env",
            "workspace",
            "export",
            "--mode",
            "production",
            "--output",
            ".env.production.local",
        ])
        .unwrap_err();
        assert!(error.to_string().contains("--format <FORMAT>"));
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
    fn cli_accepts_top_level_install_and_uninstall_commands() {
        let cli = Cli::try_parse_from(["shine", "install", "proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Install { target, replace_managed: false, yes: false } if target == "proxy"
        ));

        let cli =
            Cli::try_parse_from(["shine", "install", "app/starship", "--replace-managed"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Install { target, replace_managed: true, yes: false } if target == "app/starship"
        ));

        let cli = Cli::try_parse_from(["shine", "uninstall", "starship"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Uninstall {
                target,
                force: false,
                purge: false,
                dry_run: false,
                yes: false,
            } if target == "starship"
        ));
    }

    #[test]
    fn cli_rejects_top_level_shims_without_category() {
        assert!(Cli::try_parse_from(["shine", "install"]).is_err());
        assert!(Cli::try_parse_from(["shine", "reinstall"]).is_err());
        assert!(Cli::try_parse_from(["shine", "reinstall", "proxy"]).is_err());
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
        let list = Commands::List {
            available: false,
            kind: None,
        };

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
                verbose: false,
                run_generators: false
            } if target == "setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "info", "app/surge", "--run-generators"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                run_generators: true,
                ..
            } if target == "app/surge"
        ));

        let cli = Cli::try_parse_from(["shine", "info", "setproxy", "--diff"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: true,
                verbose: false,
                run_generators: false
            } if target == "setproxy"
        ));

        let cli = Cli::try_parse_from(["shine", "info", "setproxy", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: false,
                verbose: true,
                run_generators: false
            } if target == "setproxy"
        ));

        let cli =
            Cli::try_parse_from(["shine", "info", "setproxy", "--diff", "--verbose"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Info {
                target,
                diff: true,
                verbose: true,
                run_generators: false
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

        assert!(Cli::try_parse_from(["shine", "env", "secret", "identity", "list"]).is_ok());
        assert!(Cli::try_parse_from(["shine", "env", "identity", "list"]).is_err());
    }

    #[test]
    fn cli_rejects_legacy_show_command() {
        let err = Cli::try_parse_from(["shine", "show", "setproxy"]).unwrap_err();
        assert!(err.to_string().contains("unrecognized subcommand 'show'"));
    }

    #[test]
    fn cli_accepts_primary_authoring_artifact_and_bootstrap_spellings() {
        assert!(matches!(
            Cli::try_parse_from(["shine", "preset", "new", "app"])
                .unwrap()
                .command,
            Commands::Preset {
                command: PresetCommands::New {
                    kind: PresetTemplateKind::App,
                    force: false,
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["shine", "preset", "new", "sys"])
                .unwrap()
                .command,
            Commands::Preset {
                command: PresetCommands::New {
                    kind: PresetTemplateKind::Sys,
                    force: false,
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["shine", "app", "artifact", "apply", "surge"])
                .unwrap()
                .command,
            Commands::App {
                command: AppCommands::Artifact {
                    command: AppArtifactCommands::Apply { app_id, .. }
                }
            } if app_id == "surge"
        ));
        assert!(matches!(
            Cli::try_parse_from(["shine", "sys", "bootstrap", "--dry-run"])
                .unwrap()
                .command,
            Commands::Sys {
                command: SysCommands::Bootstrap { dry_run: true, .. }
            }
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
                    ..
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
                    ..
                }
            } if category == "surge" && file == "subscription-proxies.conf"
        ));
    }

    #[test]
    fn cli_accepts_explicit_app_recovery() {
        let cli = Cli::try_parse_from(["shine", "app", "recover"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Recover { yes: false }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "app", "recover", "--yes"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::App {
                command: AppCommands::Recover { yes: true }
            }
        ));
    }

    #[test]
    fn cli_accepts_explicit_shell_recovery() {
        let cli = Cli::try_parse_from(["shine", "shell", "recover"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Recover { yes: false }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "shell", "recover", "--yes"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Shell {
                command: ShellCommands::Recover { yes: true }
            }
        ));
    }

    #[test]
    fn cli_accepts_explicit_sys_recovery() {
        let cli = Cli::try_parse_from(["shine", "sys", "recover"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Recover { yes: false }
            }
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "recover", "--yes"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Recover { yes: true }
            }
        ));
    }

    #[test]
    fn cli_rejects_removed_pre_1_0_compatibility_commands() {
        for args in [
            &["shine", "reinstall", "proxy"][..],
            &["shine", "shell", "reinstall", "proxy"],
            &["shine", "app", "reinstall", "ghostty"],
            &["shine", "shell", "init"],
            &["shine", "app", "init"],
            &["shine", "app", "build", "surge"],
            &["shine", "app", "unbuild", "surge"],
            &["shine", "sys", "init"],
            &["shine", "env", "encrypt"],
            &["shine", "env", "decrypt", "TOKEN_SECRET"],
            &["shine", "env", "export", "TOKEN"],
            &["shine", "env", "seal"],
            &["shine", "env", "identity", "list"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_err(),
                "accepted removed command: {args:?}"
            );
        }
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
    fn cli_accepts_sys_bootstrap_options() {
        let cli = Cli::try_parse_from(["shine", "sys", "bootstrap"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: None,
                    dry_run: false,
                    force_profile: false,
                    proxy: false,
                    ..
                }
            } if items.is_empty()
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "bootstrap", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: None,
                    dry_run: true,
                    force_profile: false,
                    proxy: false,
                    ..
                }
            } if items.is_empty()
        ));

        let cli =
            Cli::try_parse_from(["shine", "sys", "bootstrap", "--preset", "recommended"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: Some(ref preset),
                    dry_run: false,
                    force_profile: false,
                    proxy: false,
                    ..
                }
            } if items.is_empty() && preset == "recommended"
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "bootstrap", "--force-profile"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: None,
                    dry_run: false,
                    force_profile: true,
                    proxy: false,
                    ..
                }
            } if items.is_empty()
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "bootstrap", "--proxy"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: None,
                    dry_run: false,
                    force_profile: false,
                    proxy: true,
                    ..
                }
            } if items.is_empty()
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "sys",
            "bootstrap",
            "--preset",
            "recommended",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: Some(ref preset),
                    dry_run: true,
                    force_profile: false,
                    proxy: false,
                    ..
                }
            } if items.is_empty() && preset == "recommended"
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "bootstrap", "rust", "mise"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    preset: None,
                    ..
                }
            } if items == ["rust", "mise"]
        ));

        let cli = Cli::try_parse_from([
            "shine",
            "sys",
            "bootstrap",
            "--item",
            "rust",
            "--item",
            "mise",
            "--yes",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Bootstrap {
                    items,
                    exact_items,
                    yes: true,
                    ..
                }
            } if items.is_empty() && exact_items == ["rust", "mise"]
        ));

        assert!(
            Cli::try_parse_from([
                "shine",
                "sys",
                "bootstrap",
                "mise",
                "--preset",
                "recommended",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["shine", "sys", "bootstrap", "rust", "--item", "mise",]).is_err()
        );
        assert!(Cli::try_parse_from(["shine", "sys", "bootstrap", "--dry-run", "--yes",]).is_err());
    }

    #[test]
    fn cli_accepts_sys_profile_state_commands() {
        let cli = Cli::try_parse_from(["shine", "sys", "profile", "disable", "mise", "--dry-run"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Profile {
                    command: SysProfileCommands::Disable {
                        ref item,
                        dry_run: true,
                        ..
                    }
                }
            } if item == "mise"
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
    fn cli_rejects_removed_sys_update_command() {
        assert!(Cli::try_parse_from(["shine", "sys", "update"]).is_err());
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
                    dry_run: true,
                    yes: false
                }
            } if item == "split-dns"
        ));

        let cli = Cli::try_parse_from(["shine", "sys", "uninstall", "split-dns"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sys {
                command: SysCommands::Uninstall {
                    ref item,
                    dry_run: false,
                    yes: false
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
