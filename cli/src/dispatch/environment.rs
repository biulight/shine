//! Env command dispatch.
use super::*;

pub(super) async fn dispatch(config: &Config, command: EnvCommands) -> Result<()> {
    match command {
        EnvCommands::List { reveal } => env::commands::handle_list(config, reveal).await,
        EnvCommands::Set { key, value, target } => {
            env::commands::handle_set(config, &key, &value, &target).await
        }
        EnvCommands::Delete { key, target } => {
            env::commands::handle_delete(config, &key, &target).await
        }
        EnvCommands::Explain { key, reveal } => {
            env::commands::handle_explain(config, &key, reveal).await
        }
        EnvCommands::Get { key } => env::commands::handle_get(config, &key).await,
        EnvCommands::Run(cmd) => {
            if !cmd.file_groups.is_empty() {
                return env::workspace::file_groups::handle_run(config, &cmd).await;
            }
            env::workspace::handle_run(
                config,
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
                    config,
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
                        config,
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
                        config,
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
                        config,
                        &name,
                        &workspace,
                        &mode,
                        &release,
                        release_all_declared,
                        &command,
                    )
                    .await
                }
                EnvBrokerPolicySubcommand::List => env::broker::handle_policy_list(config).await,
                EnvBrokerPolicySubcommand::Info { name } => {
                    env::broker::handle_policy_info(config, &name).await
                }
                EnvBrokerPolicySubcommand::Remove { name } => {
                    env::broker::handle_policy_remove(config, &name).await
                }
            },
        },
        EnvCommands::Proxy(cmd) => match cmd.command {
            EnvProxySubcommand::Install {
                command,
                with,
                project,
            } => env::proxy::install(config, &command, &with, project).await,
            EnvProxySubcommand::List => env::proxy::list(config).await,
            EnvProxySubcommand::Uninstall { command } => {
                env::proxy::uninstall(config, &command).await
            }
            EnvProxySubcommand::Enable { command, project } => {
                env::proxy::set_enabled(config, &command, true, project).await
            }
            EnvProxySubcommand::Disable { command, project } => {
                env::proxy::set_enabled(config, &command, false, project).await
            }
            EnvProxySubcommand::Exec {
                target,
                command,
                args,
            } => env::proxy::exec(config, &target, &command, &args).await,
        },
        EnvCommands::Secret(cmd) => match cmd.command {
            EnvSecretSubcommand::Decrypt { key } => {
                env::commands::handle_decrypt(config, &key).await
            }
            EnvSecretSubcommand::Export { key, alias } => {
                env::commands::handle_export(config, &key, alias.as_deref()).await
            }
            EnvSecretSubcommand::Encrypt(cmd) => {
                env::commands::handle_encrypt(
                    config,
                    cmd.backend.as_deref(),
                    &cmd.recipients,
                    cmd.set.as_deref(),
                    cmd.from.as_deref(),
                    &cmd.target,
                )
                .await
            }
            EnvSecretSubcommand::Seal(cmd) => {
                if !cmd.file_groups.is_empty() {
                    return env::workspace::file_groups::handle_seal(config, &cmd).await;
                }
                env::workspace::handle_seal(
                    config,
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
                    phone,
                    recipient_type,
                    label,
                    transport,
                    adb_serial,
                    access_control,
                    output,
                    force,
                } => {
                    if phone {
                        env::identity::handle_phone_identity_init(
                            config,
                            recipient_type.unwrap_or_default(),
                            label.as_deref(),
                            transport.unwrap_or_default().as_str(),
                            adb_serial.as_deref(),
                        )
                        .await
                    } else {
                        env::identity::handle_identity_init(
                            config,
                            touch_id,
                            access_control.as_deref(),
                            output.as_deref(),
                            force,
                        )
                        .await
                    }
                }
                EnvIdentitySubcommand::List => env::identity::handle_identity_list(config).await,
            },
        },
    }
}
