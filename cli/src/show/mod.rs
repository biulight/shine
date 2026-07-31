mod collect;
mod render;
mod resolve;

use crate::config::Config;
use crate::status::FileStatus;
use crate::{colors, path_display};
use anyhow::{Result, bail};
use resolve::ShowRef;

pub(crate) struct UpdateDiffs {
    app_files: Vec<collect::AppShowFile>,
    shell_files: Vec<collect::ShellShowFile>,
}

impl UpdateDiffs {
    pub(crate) async fn collect(config: &Config) -> Result<Self> {
        Ok(Self {
            app_files: collect::collect_app_files(config).await?,
            shell_files: collect::collect_shell_files(config).await?,
        })
    }

    pub(crate) async fn print_shell_for_row(&self, config: &Config, label: &str) -> Result<()> {
        for file in self.shell_files.iter().filter(|file| {
            file.status == "update available"
                && format!("{}/{}", file.category.name, file.file.command_name) == label
        }) {
            render::print_shell_update_diff(config, file).await?;
        }
        Ok(())
    }

    pub(crate) async fn print_app_for_row(&self, config: &Config, label: &str) -> Result<()> {
        for file in self.app_files.iter().filter(|file| {
            if file.status != FileStatus::UpdateAvail {
                return false;
            }
            if file.category.has_explicit_files
                && file.category.list_mode == crate::apps::AppListMode::Files
            {
                app_file_label(file) == label
            } else {
                file.category.name == label
            }
        }) {
            render::print_app_update_diff(config, file).await?;
        }
        Ok(())
    }
}

fn app_file_label(file: &collect::AppShowFile) -> String {
    file.file
        .display_name
        .clone()
        .unwrap_or_else(|| format!("{}/{}", file.category.name, file.file.source_rel.display()))
}

pub async fn handle_update_target(config: &Config, target: &str) -> Result<()> {
    crate::config::print_presets_note(config);
    let diffs = UpdateDiffs::collect(config).await?;

    if diffs.app_files.is_empty() && diffs.shell_files.is_empty() {
        bail!("nothing installed yet. Run `shine shell install` or `shine app install`.");
    }

    let candidates = resolve::build_candidates(&diffs.app_files, &diffs.shell_files);
    let refs = resolve::resolve_target(target, &candidates)?;
    let mut printed = false;

    for item in refs {
        match item {
            ShowRef::AppCategory(category) => {
                let mut files = diffs
                    .app_files
                    .iter()
                    .filter(|file| {
                        file.category.name == category && file.status == FileStatus::UpdateAvail
                    })
                    .collect::<Vec<_>>();
                files.sort_by_key(|file| file.file.source_rel.clone());
                for file in files {
                    print_update_separator(printed);
                    print_app_update_row(config, file);
                    render::print_app_update_diff(config, file).await?;
                    printed = true;
                }
            }
            ShowRef::AppFile { category, source } => {
                if let Some(file) = diffs.app_files.iter().find(|file| {
                    file.category.name == category
                        && file.file.source_rel == source
                        && file.status == FileStatus::UpdateAvail
                }) {
                    print_app_update_row(config, file);
                    render::print_app_update_diff(config, file).await?;
                    printed = true;
                }
            }
            ShowRef::ShellCategory(category) => {
                let mut files = diffs
                    .shell_files
                    .iter()
                    .filter(|file| {
                        file.category.name == category && file.status == "update available"
                    })
                    .collect::<Vec<_>>();
                files.sort_by_key(|file| file.file.command_name.clone());
                for file in files {
                    print_update_separator(printed);
                    print_shell_update_row(file);
                    render::print_shell_update_diff(config, file).await?;
                    printed = true;
                }
            }
            ShowRef::ShellFile { category, command } => {
                if let Some(file) = diffs.shell_files.iter().find(|file| {
                    file.category.name == category
                        && file.file.command_name == command
                        && file.status == "update available"
                }) {
                    print_shell_update_row(file);
                    render::print_shell_update_diff(config, file).await?;
                    printed = true;
                }
            }
        }
    }

    if !printed {
        println!(
            "{}",
            colors::dim(&format!("No update available for {target}."))
        );
    }

    Ok(())
}

fn print_update_separator(printed: bool) {
    if printed {
        println!();
    }
}

fn print_shell_update_row(file: &collect::ShellShowFile) {
    println!(
        "  {}  {}/{}  {}",
        colors::symbol("↑"),
        file.category.name,
        file.file.command_name,
        colors::status_label("update available", "↑"),
    );
}

fn print_app_update_row(config: &Config, file: &collect::AppShowFile) {
    println!(
        "  {}  {}  {}  {}  {}",
        colors::symbol("↑"),
        app_file_label(file),
        colors::dim("→"),
        colors::dim(&path_display::format_home(
            &file.destination,
            &config.home_dir
        )),
        colors::status_label("update available", "↑"),
    );
}

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
