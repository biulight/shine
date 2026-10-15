//! App command dispatch.
use super::*;

pub(super) async fn dispatch(config: &Config, command: AppCommands) -> Result<()> {
    match command {
        AppCommands::List => Box::pin(apps::handle_list(config)).await,
        AppCommands::Info {
            category,
            run_generators,
            diff,
        } => Box::pin(apps::handle_info(config, &category, run_generators, diff)).await,
        AppCommands::Install {
            category,
            dry_run,
            replace_managed,
            yes,
            ..
        } => {
            Box::pin(apps::handle_install_approved(
                config,
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
            ..
        } => {
            Box::pin(apps::handle_refresh_approved(
                config,
                &category,
                file.as_deref(),
                force,
                yes,
            ))
            .await
        }
        AppCommands::Recover { yes, .. } => {
            Box::pin(apps::handle_recover_approved(config, yes)).await
        }
        AppCommands::Uninstall {
            category,
            force,
            purge,
            dry_run,
            yes,
            ..
        } => {
            Box::pin(apps::handle_uninstall_approved(
                config,
                category.as_deref(),
                force,
                purge,
                dry_run,
                yes,
            ))
            .await
        }
        AppCommands::Artifact { command } => match command {
            AppArtifactCommands::Apply { app_id, yes, .. } => {
                Box::pin(apps::handle_build_approved(config, &app_id, yes)).await
            }
            AppArtifactCommands::Remove { app_id, yes, .. } => {
                Box::pin(apps::handle_unbuild_approved(config, &app_id, yes)).await
            }
        },
    }
}
