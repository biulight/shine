//! Commands that must remain reachable without initializing runtime configuration.
use super::*;

pub(super) async fn dispatch(cli: &Cli) -> Result<std::ops::ControlFlow<()>> {
    if let Commands::MaintenanceWorker { scope, ticket } = &cli.command {
        cli::maintenance::handle_worker(scope, *ticket).await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::State {
        command: StateCommands::Check,
    } = &cli.command
    {
        cli::maintenance::handle_check().await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Clear {
        command: ClearCommands::Serve,
    } = &cli.command
    {
        // Cleanup must remain reachable after an upgrade, even with invalid or retired config.
        cli::legacy_serve::handle_uninstall().await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Init(cmd) = &cli.command {
        cli::init::handle_init(cmd.yes).await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::State {
        command: StateCommands::Migrate(cmd),
    } = &cli.command
    {
        // State migration must be able to read retired config fields before
        // normal config loading rejects them, and applies its own writes.
        let config = Box::pin(Config::load_global_runtime_for_dry_run()).await?;
        Box::pin(state::handle_migrate(&config, cmd.dry_run)).await?;
        return Ok(std::ops::ControlFlow::Break(()));
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
        theme::handle_sync(*auto, *quiet).await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command: PresetCommands::Schema { format },
    } = &cli.command
    {
        cli::preset_schema::handle_schema(*format)?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command: PresetCommands::Validate { path, format },
    } = &cli.command
    {
        let valid = cli::preset_validation::handle_validate(path, *format).await?;
        if !valid {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command:
            PresetCommands::Lint {
                path,
                format,
                deny_warnings,
            },
    } = &cli.command
    {
        let accepted = cli::preset_lint::handle_lint(path, *format, *deny_warnings).await?;
        if !accepted {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command:
            PresetCommands::Plan {
                path,
                platform,
                format,
            },
    } = &cli.command
    {
        let valid = cli::preset_authoring::handle_plan(path, *platform, *format).await?;
        if !valid {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command: PresetCommands::Test { path, format },
    } = &cli.command
    {
        let passed = cli::preset_test::handle_test(path, *format).await?;
        if !passed {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command:
            PresetCommands::Pack {
                path,
                output,
                force,
                format,
            },
    } = &cli.command
    {
        let packed = cli::preset_pack::handle_pack(path, output, *force, *format).await?;
        if !packed {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    if let Commands::Preset {
        command:
            PresetCommands::Migrate {
                path,
                dry_run,
                yes,
                format,
            },
    } = &cli.command
    {
        let migrated =
            cli::preset_migration::handle_migrate(path.as_deref(), *dry_run, *yes, *format).await?;
        if !migrated {
            std::process::exit(1);
        }
        return Ok(std::ops::ControlFlow::Break(()));
    }

    // Git overlay scope must be checked before runtime initialization can write config.
    if let Commands::Preset {
        command:
            PresetCommands::Overlay {
                command: OverlayCommands::Link(command),
            },
    } = &cli.command
        && command.git.is_some()
    {
        let config = Box::pin(Config::load_read_only()).await?;
        Box::pin(handle_overlay_link(&config, command.clone())).await?;
        return Ok(std::ops::ControlFlow::Break(()));
    }

    // Pure env inspection and directed writes must not initialize state, run
    // advisory workers, sync overlays, or add output to `env get`.
    if let Commands::Env { command } = &cli.command {
        match command {
            EnvCommands::List { reveal } => {
                env::commands::handle_list(&Config::load_read_only().await?, *reveal).await?;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            EnvCommands::Get { key } => {
                env::commands::handle_get(&Config::load_read_only().await?, key).await?;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            EnvCommands::Explain { key, reveal } => {
                env::commands::handle_explain(&Config::load_read_only().await?, key, *reveal)
                    .await?;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            EnvCommands::Set { key, value, target } => {
                env::commands::handle_set(&Config::load_read_only().await?, key, value, target)
                    .await?;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            EnvCommands::Delete { key, target } => {
                env::commands::handle_delete(&Config::load_read_only().await?, key, target).await?;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            EnvCommands::Secret(cmd) => {
                if let EnvSecretSubcommand::Encrypt(cmd) = &cmd.command {
                    env::commands::handle_encrypt(
                        &Config::load_read_only().await?,
                        cmd.backend.as_deref(),
                        &cmd.recipients,
                        cmd.set.as_deref(),
                        cmd.from.as_deref(),
                        &cmd.target,
                    )
                    .await?;
                    return Ok(std::ops::ControlFlow::Break(()));
                }
            }
            _ => {}
        }
    }

    Ok(std::ops::ControlFlow::Continue(()))
}
