use super::metadata;
use crate::colors;
use crate::config::Config;
use crate::output;
use crate::path_display;
use anyhow::Result;
use std::ffi::{OsStr, OsString};
use std::path::Path;

pub(super) fn build_link_specs(
    config: &Config,
    categories: &[metadata::ShellCategory],
) -> Result<Vec<crate::bin_links::LinkSpec>> {
    categories
        .iter()
        .flat_map(|cat| {
            cat.files.iter().map(|file| {
                let source =
                    super::deployment::deployment_source_path(config, &cat.name, &file.source_rel);
                let rendered =
                    super::deployment::rendered_path(config, &cat.name, &file.source_rel);
                let has_transforms = !file.transforms.is_empty()
                    || std::fs::read(&source)
                        .ok()
                        .is_some_and(|bytes| crate::presets::parse_template_annotation(&bytes));
                let effective = if has_transforms { rendered } else { source };
                let bun_runtime = super::deployment::bun_runtime_spec(config, &cat.name, file)?;
                Ok(crate::bin_links::LinkSpec {
                    source: effective,
                    link_name: OsString::from(&file.command_name),
                    runtime: file.runtime,
                    bun_dependencies: bun_runtime.dependency_mode,
                    env: file.env.iter().map(|spec| spec.to_with_arg()).collect(),
                    render_target: (config.is_external_presets
                        && config.external_shell_mode == crate::config::ExternalShellMode::Live
                        && has_transforms)
                        .then(|| format!("shell/{}/{}", cat.name, file.command_name)),
                })
            })
        })
        .collect()
}

pub(super) fn print_link_conflicts(
    config: &Config,
    conflicts: &[crate::bin_links::LinkConflict],
    category_hint: Option<&str>,
) {
    for conflict in conflicts {
        let lines = link_conflict_detail_lines(config, conflict, category_hint);
        if let Some((first, rest)) = lines.split_first() {
            let command = conflict
                .link_path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("unknown");
            output::detail_line(
                "Link Conflict",
                &colors::yellow(command),
                Some(first.clone()),
            );
            for line in rest {
                println!("{}{}", " ".repeat(16), colors::dim(line));
            }
        }
    }
}

fn link_conflict_detail_lines(
    config: &Config,
    conflict: &crate::bin_links::LinkConflict,
    category_hint: Option<&str>,
) -> Vec<String> {
    let link = path_display::format_home(&conflict.link_path, &config.home_dir);
    let wanted = path_display::format_home(&conflict.source, &config.home_dir);
    let mut lines = Vec::new();

    match conflict.kind {
        crate::bin_links::LinkConflictKind::DuplicateName => {
            lines.push("duplicate requested command".to_string());
        }
        crate::bin_links::LinkConflictKind::ExistingEntry => {
            lines.push(format!(
                "existing: {}",
                describe_existing_link(&conflict.link_path, &link)
            ));
        }
    }

    lines.push(format!("wanted:   {wanted}"));
    lines.push(format!(
        "fix:      remove {link} or run `{}`",
        reinstall_command(config, category_hint, &conflict.source)
    ));
    lines
}

fn describe_existing_link(path: &Path, display_path: &str) -> String {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => match std::fs::read_link(path) {
            Ok(target) => format!("{display_path} -> {}", path_display::format(&target)),
            Err(_) => format!("{display_path} -> unreadable target"),
        },
        Ok(meta) if meta.is_dir() => format!("directory {display_path}"),
        Ok(_) => format!("file {display_path}"),
        Err(_) => format!("{display_path} no longer exists"),
    }
}

fn reinstall_command(config: &Config, category_hint: Option<&str>, source: &Path) -> String {
    if let Some(category) = category_hint {
        return format!("shine install shell/{category} --replace-managed");
    }
    if let Some(category) = infer_shell_category(config, source) {
        return format!("shine install shell/{category} --replace-managed");
    }
    "shine shell install --replace-managed".to_string()
}

fn infer_shell_category(config: &Config, source: &Path) -> Option<String> {
    shell_category_after_root(source.strip_prefix(config.presets_dir()).ok()?)
        .or_else(|| shell_category_after_root(source.strip_prefix(config.rendered_dir()).ok()?))
}

fn shell_category_after_root(path: &Path) -> Option<String> {
    let mut components = path.components();
    if components.next()?.as_os_str() != OsStr::new("shell") {
        return None;
    }
    components.next()?.as_os_str().to_str().map(str::to_string)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell").await
    }

    fn config_with_deepseek_key(dir: &Path) -> Config {
        let mut config = Config::new_for_test(dir);
        config
            .env
            .insert("DEEPSEEK_API_KEY".into(), "test-deepseek-key".into());
        config
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn link_conflict_detail_describes_existing_file() {
        let dir = make_temp_dir().await;
        let config = config_with_deepseek_key(&dir);
        tokio::fs::create_dir_all(config.bin_dir()).await.unwrap();
        let link_path = config.bin_dir().join("setproxy");
        let source = config.presets_dir().join("shell/proxy/set_proxy.sh");
        tokio::fs::write(&link_path, b"custom command")
            .await
            .unwrap();

        let conflict = crate::bin_links::LinkConflict {
            link_path: link_path.clone(),
            source: source.clone(),
            kind: crate::bin_links::LinkConflictKind::ExistingEntry,
        };
        let lines = link_conflict_detail_lines(&config, &conflict, Some("proxy"));

        assert_eq!(lines[0], "existing: file ~/bin/setproxy");
        assert_eq!(lines[1], "wanted:   ~/presets/shell/proxy/set_proxy.sh");
        assert_eq!(
            lines[2],
            "fix:      remove ~/bin/setproxy or run `shine install shell/proxy --replace-managed`"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn link_conflict_detail_describes_stale_symlink() {
        let dir = make_temp_dir().await;
        let config = config_with_deepseek_key(&dir);
        tokio::fs::create_dir_all(config.bin_dir()).await.unwrap();
        tokio::fs::create_dir_all(config.presets_dir().join("shell/proxy"))
            .await
            .unwrap();
        let link_path = config.bin_dir().join("setproxy");
        let source = config.presets_dir().join("shell/proxy/set_proxy.sh");
        let other = dir.join("other-setproxy");
        tokio::fs::write(&other, b"#!/bin/sh\n").await.unwrap();
        tokio::fs::symlink(&other, &link_path).await.unwrap();

        let conflict = crate::bin_links::LinkConflict {
            link_path,
            source,
            kind: crate::bin_links::LinkConflictKind::ExistingEntry,
        };
        let lines = link_conflict_detail_lines(&config, &conflict, None);

        assert!(lines[0].starts_with("existing: ~/bin/setproxy -> "));
        assert!(lines[0].contains("other-setproxy"));
        assert_eq!(lines[1], "wanted:   ~/presets/shell/proxy/set_proxy.sh");
        assert_eq!(
            lines[2],
            "fix:      remove ~/bin/setproxy or run `shine install shell/proxy --replace-managed`"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
