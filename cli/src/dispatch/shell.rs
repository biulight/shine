//! Shell command dispatch.
use super::*;

pub(super) async fn dispatch(config: &Config, command: ShellCommands) -> Result<()> {
    match command {
        ShellCommands::List => Box::pin(shells::handle_list(config)).await,
        ShellCommands::Info { target } => Box::pin(shells::handle_info(config, &target)).await,
        ShellCommands::Recover { yes, .. } => {
            Box::pin(shells::handle_recover_approved(config, yes)).await
        }
        ShellCommands::Install {
            target,
            dry_run: true,
            replace_managed: _,
            yes: _,
            ..
        } => Box::pin(shells::handle_install_dry_run(config, target.as_deref())).await,
        ShellCommands::Install {
            target,
            dry_run: false,
            replace_managed,
            yes,
            ..
        } => {
            Box::pin(shells::handle_install_approved(
                config,
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
            ..
        } => {
            Box::pin(shells::handle_uninstall_approved(
                config,
                target.as_deref(),
                purge,
                dry_run,
                yes,
            ))
            .await
        }
    }
}
