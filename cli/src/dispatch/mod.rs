//! CLI dispatch with explicit pre-initialization and configured phases.
mod app;
mod before_config;
mod environment;
mod preset;
mod shell;
mod system;

use anyhow::{Result, bail};
#[cfg(test)]
use clap::Parser;
#[cfg(test)]
use cli::{commands::CompletionShell, completion};

use crate as cli;
use crate::{
    apps, colors, commands, config, env, git_pull, info, list, shells, ssh, state, sys, task,
    theme, update_check,
};

use commands::{
    AppArtifactCommands, AppCommands, ClearCommands, Cli, Commands, CompletionCommands,
    EnvBrokerPolicySubcommand, EnvBrokerSubcommand, EnvCommands, EnvIdentitySubcommand,
    EnvProxySubcommand, EnvSecretSubcommand, EnvWorkspaceSubcommand, LocalCommands,
    OverlayCommands, PresetCommands, PresetTemplateKind, ResourceKind, SelfCommands, ShellCommands,
    StateCommands, SysCommands, SysProfileCommands, TaskCommands, ThemeCommands, TrustCommands,
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

pub async fn run(cli: Cli) -> Result<()> {
    cli::with_full_upgrade_plan(
        cli.command.full_upgrade_plan(),
        cli::with_security_plan_verbosity(cli.command.security_plan_verbose(), run_command(cli)),
    )
    .await
}

async fn run_command(cli: Cli) -> Result<()> {
    if let std::ops::ControlFlow::Break(()) = before_config::dispatch(&cli).await? {
        return Ok(());
    }

    cli::maintenance::maybe_notify(&cli.command).await;

    let config = if matches!(cli.command, Commands::Env { .. }) {
        Box::pin(Config::load_read_only()).await?
    } else {
        Box::pin(Config::load_or_init()).await?
    };

    if config.legacy_allow_app_hooks || config.legacy_allow_sys_code {
        eprintln!(
            "Warning: allow_app_hooks/allow_sys_code are retired and ignored; review current external code with `shine trust inspect <TARGET>` and enroll it with `shine trust grant <TARGET>`."
        );
    }

    if let Commands::ShellLaunch { target, args } = &cli.command {
        return shells::handle_launch_live(&config, target, args).await;
    }

    if let Commands::ShellRender { target } = &cli.command {
        return shells::handle_render_live(&config, target).await;
    }

    warn_if_runtime_schema_pending(&cli.command).await;

    update_check::maybe_notify(&config, &cli.command).await?;

    match cli.command {
        Commands::MaintenanceWorker { .. } => {
            unreachable!("maintenance worker routes before config loading")
        }
        Commands::Clear { .. } => unreachable!("legacy cleanup routes before config loading"),
        Commands::ShellRender { .. } | Commands::ShellLaunch { .. } => unreachable!(),
        Commands::Init(_) => unreachable!(),
        Commands::Completions {
            command: CompletionCommands::Install,
        } => Box::pin(shells::handle_completion_install(&config)).await,
        Commands::Completions { .. } => unreachable!(),
        Commands::State { .. } => unreachable!(),
        Commands::Theme { .. } => unreachable!(),
        Commands::Trust { command } => match command {
            TrustCommands::List { verbose } => cli::trust::handle_list(&config, verbose).await,
            TrustCommands::Inspect { target } => cli::trust::handle_inspect(&config, &target).await,
            TrustCommands::Grant {
                target,
                yes,
                development,
            } => cli::trust::handle_grant(&config, &target, yes, development).await,
            TrustCommands::Revoke { target } => cli::trust::handle_revoke(&config, &target).await,
        },
        Commands::Install {
            target,
            replace_managed,
            yes,
            ..
        } => handle_install_shim_approved(&config, &target, replace_managed, yes).await,
        Commands::Uninstall {
            target,
            force,
            dry_run,
            yes,
            ..
        } => handle_uninstall_shim_approved(&config, &target, force, dry_run, yes).await,
        Commands::App { command } => app::dispatch(&config, command).await,
        Commands::Update(cmd) => {
            let config = if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                Box::pin(Config::load_or_init()).await?
            } else {
                config
            };
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
        Commands::Upgrade(cmd) => {
            let config = if cmd.pull {
                git_pull::handle_pull(&config, cmd.verbose).await?;
                Box::pin(Config::load_or_init()).await?
            } else {
                config
            };
            handle_config_upgrade(
                &config,
                cmd.target.as_deref(),
                cmd.verbose,
                cmd.prune_stale,
                cmd.yes,
            )
            .await
        }
        Commands::Preset { command } => preset::dispatch(&config, command).await,
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
        Commands::Shell { command } => shell::dispatch(&config, command).await,
        Commands::Env { command } => environment::dispatch(&config, command).await,
        Commands::Sys { command } => system::dispatch(&config, command).await,
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
mod tests;
