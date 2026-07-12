mod collect;
mod render;
mod resolve;

use crate::config::Config;
use anyhow::{Result, bail};
use resolve::ShowRef;

pub async fn handle_show(config: &Config, target: &str, diff: bool, verbose: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let app_files = collect::collect_app_files(config).await?;
    let shell_files = collect::collect_shell_files(config).await?;

    if app_files.is_empty() && shell_files.is_empty() {
        bail!("nothing installed yet. Run `shine shell install` or `shine app install`.");
    }

    let candidates = resolve::build_candidates(&app_files, &shell_files);
    let refs = resolve::resolve_target(target, &candidates)?;

    let mut first = true;
    for item in refs {
        if !first {
            println!();
        }
        first = false;
        match item {
            ShowRef::AppCategory(category) => {
                let mut files: Vec<_> = app_files
                    .iter()
                    .filter(|f| f.category.name == category)
                    .cloned()
                    .collect();
                files.sort_by_key(|f| f.file.source_rel.clone());
                for (index, file) in files.iter().enumerate() {
                    if index > 0 {
                        println!();
                    }
                    render::print_app_file(config, file, diff, verbose).await?;
                }
            }
            ShowRef::AppFile { category, source } => {
                let file = app_files
                    .iter()
                    .find(|f| f.category.name == category && f.file.source_rel == source)
                    .ok_or_else(|| anyhow::anyhow!("installed app config not found"))?;
                render::print_app_file(config, file, diff, verbose).await?;
            }
            ShowRef::ShellCategory(category) => {
                let mut files: Vec<_> = shell_files
                    .iter()
                    .filter(|f| f.category.name == category)
                    .cloned()
                    .collect();
                files.sort_by_key(|f| f.file.command_name.clone());
                for (index, file) in files.iter().enumerate() {
                    if index > 0 {
                        println!();
                    }
                    render::print_shell_file(config, file, diff, verbose).await?;
                }
            }
            ShowRef::ShellFile { category, command } => {
                let file = shell_files
                    .iter()
                    .find(|f| f.category.name == category && f.file.command_name == command)
                    .ok_or_else(|| anyhow::anyhow!("installed shell preset not found"))?;
                render::print_shell_file(config, file, diff, verbose).await?;
            }
        }
    }

    Ok(())
}
