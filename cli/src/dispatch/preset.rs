//! Preset command dispatch.
use super::*;

pub(super) async fn dispatch(config: &Config, command: PresetCommands) -> Result<()> {
    match command {
        PresetCommands::New {
            kind,
            unrestricted,
            force,
        } => match kind {
            PresetTemplateKind::App => apps::handle_init_template(force, unrestricted).await,
            PresetTemplateKind::Shell => shells::handle_init_template(force, unrestricted).await,
            PresetTemplateKind::Sys => sys::handle_init_template(force, unrestricted).await,
        },
        PresetCommands::Schema { .. } => unreachable!(),
        PresetCommands::Validate { .. } => unreachable!(),
        PresetCommands::Lint { .. } => unreachable!(),
        PresetCommands::Plan { .. } => unreachable!(),
        PresetCommands::Test { .. } => unreachable!(),
        PresetCommands::Pack { .. } => unreachable!(),
        PresetCommands::Migrate { .. } => unreachable!(),
        PresetCommands::Export(cmd) => {
            Box::pin(handle_preset_export(config, cmd.dir, cmd.force)).await
        }
        PresetCommands::Copy(cmd) => Box::pin(handle_preset_copy(&cmd.target, cmd.force)).await,
        PresetCommands::Link(cmd) => {
            Box::pin(handle_preset_link(config, cmd.path, cmd.create, cmd.live)).await
        }
        PresetCommands::Unlink => Box::pin(handle_preset_unlink(config)).await,
        PresetCommands::Overlay { command } => match command {
            OverlayCommands::Link(cmd) => Box::pin(handle_overlay_link(config, cmd)).await,
            OverlayCommands::Unlink => Box::pin(handle_overlay_unlink(config)).await,
            OverlayCommands::Info => handle_overlay_info(config),
        },
        PresetCommands::Pull => git_pull::handle_pull(config, false).await,
    }
}
