//! Sys command dispatch.
use super::*;

pub(super) async fn dispatch(config: &Config, command: SysCommands) -> Result<()> {
    match command {
        SysCommands::Recover { yes, .. } => {
            Box::pin(sys::handle_recover_approved(config, yes)).await
        }
        SysCommands::List { all } => Box::pin(sys::handle_list(config, all)).await,
        SysCommands::Info { item } => Box::pin(sys::handle_info(config, &item)).await,
        SysCommands::Status => Box::pin(sys::handle_status(config)).await,
        SysCommands::Bootstrap {
            items,
            exact_items,
            preset,
            dry_run,
            force_profile,
            proxy,
            yes,
            verbose,
        } => {
            let requested = if exact_items.is_empty() {
                items
            } else {
                exact_items
            };
            Box::pin(sys::handle_init(
                config,
                sys::BootstrapCliOptions {
                    requested: &requested,
                    preset: preset.as_deref(),
                    dry_run,
                    force_profile,
                    proxy,
                    yes,
                    verbose,
                },
            ))
            .await
        }
        SysCommands::Profile { command } => match command {
            SysProfileCommands::Enable {
                item, dry_run, yes, ..
            } => {
                Box::pin(sys::handle_profile_enable_approved(
                    config, &item, dry_run, yes,
                ))
                .await
            }
            SysProfileCommands::Disable {
                item, dry_run, yes, ..
            } => {
                Box::pin(sys::handle_profile_disable_approved(
                    config, &item, dry_run, yes,
                ))
                .await
            }
        },
        SysCommands::Apply {
            item, dry_run, yes, ..
        } => {
            Box::pin(sys::handle_apply_approved(
                config,
                item.as_deref(),
                dry_run,
                yes,
            ))
            .await
        }
        SysCommands::Uninstall {
            item, dry_run, yes, ..
        } => Box::pin(sys::handle_uninstall_approved(config, &item, dry_run, yes)).await,
    }
}
