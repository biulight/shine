use super::collect::{AppShowFile, ShellShowFile};
use crate::apps::{AppCategory, AppFile, source_bytes_for_file};
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::path_display;
use crate::status::FileStatus;
use anyhow::{Context, Result};
use similar::TextDiff;
use std::collections::BTreeMap;
use std::path::Path;

pub(super) async fn print_app_file(
    config: &Config,
    item: &AppShowFile,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    let label = fallback_app_label(&item.category, &item.file);
    let source_path = config.preset_path(
        Path::new("app")
            .join(&item.category.name)
            .join(&item.file.source_rel),
    );

    println!("{}", colors::bold(&format!("App Config: {label}")));
    if let Some(desc) = &item.file.description {
        println!("{}  {desc}", colors::dim("Description"));
    } else if let Some(desc) = &item.category.description {
        println!("{}  {desc}", colors::dim("Description"));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        path_display::format_home(&source_path, &config.home_dir)
    );
    println!(
        "{}  {}",
        colors::dim("Destination"),
        path_display::format_home(&item.destination, &config.home_dir)
    );
    println!(
        "{}       {}",
        colors::dim("Status"),
        colored_app_status(item.status)
    );
    if !item.file.transforms.is_empty() {
        println!(
            "{}   {}",
            colors::dim("Transforms"),
            item.file.transforms.join(", ")
        );
    }
    if let Some(entry) = &item.manifest_entry {
        println!("{} {}", colors::dim("Manifest hash"), entry.content_hash);
        if let Some(backup) = &entry.backup {
            println!(
                "{}       {}",
                colors::dim("Backup"),
                path_display::format_home(backup, &config.home_dir)
            );
        }
    }
    if diff {
        let diff_output = app_diff_output(config, item).await?;
        print_block("Diff", &item.destination, &diff_output);
    }
    if verbose {
        print_file_content(&item.destination, "Content").await?;
    }
    Ok(())
}

pub(super) async fn print_shell_file(
    config: &Config,
    item: &ShellShowFile,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    let label = format!("{}/{}", item.category.name, item.file.command_name);
    println!("{}", colors::bold(&format!("Shell Preset: {label}")));
    if let Some(desc) = item.file.description.first() {
        println!("{}  {desc}", colors::dim("Description"));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        path_display::format_home(&item.source_path, &config.home_dir)
    );
    println!(
        "{}     {}",
        colors::dim("Bin link"),
        path_display::format_home(&item.link_path, &config.home_dir)
    );
    if let Some(target) = &item.link_target {
        println!(
            "{} {}",
            colors::dim("Link target"),
            path_display::format_home(target, &config.home_dir)
        );
    }
    println!(
        "{}       {}",
        colors::dim("Status"),
        colored_shell_status(item.status)
    );
    println!("{} {}", colors::dim("Needs source"), item.file.needs_source);

    let content_path = shell_content_path(item);

    if content_path == item.rendered_path {
        println!(
            "{}     {}",
            colors::dim("Rendered"),
            path_display::format_home(&item.rendered_path, &config.home_dir)
        );
    }
    if diff {
        let diff_output = shell_diff_output(config, item, &content_path).await?;
        print_block("Diff", &content_path, &diff_output);
    }
    if verbose {
        print_file_content(&content_path, "Content").await?;
    }
    Ok(())
}

pub(super) async fn print_app_update_diff(config: &Config, item: &AppShowFile) -> Result<()> {
    let diff_output = app_diff_output(config, item).await?;
    print_block("Diff", &item.destination, &diff_output);
    Ok(())
}

pub(super) async fn print_shell_update_diff(config: &Config, item: &ShellShowFile) -> Result<()> {
    let content_path = shell_content_path(item);
    let diff_output = shell_diff_output(config, item, &content_path).await?;
    print_block("Diff", &content_path, &diff_output);
    Ok(())
}

fn shell_content_path(item: &ShellShowFile) -> std::path::PathBuf {
    item.link_target
        .as_ref()
        .filter(|target| target.starts_with(&item.rendered_path) || **target == item.rendered_path)
        .cloned()
        .or_else(|| {
            item.rendered_path
                .exists()
                .then(|| item.rendered_path.clone())
        })
        .unwrap_or_else(|| item.source_path.clone())
}

fn print_heading(heading: &str, path: &Path) {
    println!();
    println!(
        "{}",
        colors::dim(&format!(
            "--- {heading}: {} ---",
            path_display::format(path)
        ))
    );
}

fn print_block(heading: &str, path: &Path, text: &str) {
    print_heading(heading, path);
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
}

async fn print_file_content(path: &Path, heading: &str) -> Result<()> {
    print_heading(heading, path);
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading installed content: {}", path.display()))?;
    print!("{}", String::from_utf8_lossy(&bytes));
    if !bytes.ends_with(b"\n") {
        println!();
    }
    Ok(())
}

fn fallback_app_label(category: &AppCategory, file: &AppFile) -> String {
    file.display_name
        .clone()
        .unwrap_or_else(|| format!("{}/{}", category.name, file.source_rel.display()))
}

fn status_parts(status: FileStatus) -> (&'static str, &'static str) {
    match status {
        FileStatus::Missing => ("destination missing", "!"),
        FileStatus::UserModified => ("user modified", "~"),
        FileStatus::UpdateAvail => ("update available", "↑"),
        FileStatus::Partial => ("partial install", "~"),
        FileStatus::UpToDate => ("up-to-date", "✓"),
        FileStatus::NotInstalled => ("not installed", "✗"),
    }
}

fn status_sym(status: FileStatus) -> &'static str {
    status_parts(status).1
}

fn colored_app_status(status: FileStatus) -> String {
    colors::status_label(status_parts(status).0, status_sym(status))
}

// status is a free-form &'static str produced by the shell subsystem; _ => "~" is intentional.
fn shell_status_sym(status: &str) -> &'static str {
    match status {
        "up-to-date" => "✓",
        "update available" => "↑",
        "preset present, bin symlink missing"
        | "bin symlink present, script missing"
        | "bin symlink present, preset missing" => "~",
        "rendered script missing" => "!",
        "not installed" => "✗",
        _ => "~",
    }
}

fn colored_shell_status(status: &str) -> String {
    colors::status_label(status, shell_status_sym(status))
}

async fn app_diff_output(config: &Config, item: &AppShowFile) -> Result<String> {
    if item
        .file
        .generator
        .as_ref()
        .is_some_and(|generator| !generator.auto)
    {
        return Ok(
            "Expected content is an explicitly refreshed generator snapshot; \
             run `shine app refresh` to materialize it without polling during show.\n"
                .to_string(),
        );
    }
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);
    let expected = match source_bytes_for_file(config, &item.category, &item.file, env_map).await {
        Some(bytes) => bytes,
        None => {
            return Ok("Unable to render expected content from the active preset.\n".to_string());
        }
    };

    let current = match tokio::fs::read(&item.destination).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!(
                "Current file is missing: {}.\n",
                item.destination.display()
            ));
        }
        Err(err) => {
            return Ok(format!(
                "Unable to read current file {}: {err}\n",
                item.destination.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        &current,
        &expected,
        &item.destination.to_string_lossy(),
        &format!(
            "expected: app/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

async fn shell_diff_output(
    config: &Config,
    item: &ShellShowFile,
    current_path: &Path,
) -> Result<String> {
    let expected = match shell_expected_bytes(config, item).await? {
        Some(bytes) => bytes,
        None => {
            return Ok(
                "Unable to render expected shell content from the active preset.\n".to_string(),
            );
        }
    };

    let current = match tokio::fs::read(current_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!(
                "Current script is missing: {}.\n",
                current_path.display()
            ));
        }
        Err(err) => {
            return Ok(format!(
                "Unable to read current script {}: {err}\n",
                current_path.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        &current,
        &expected,
        &current_path.to_string_lossy(),
        &format!(
            "expected: shell/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

async fn shell_expected_bytes(config: &Config, item: &ShellShowFile) -> Result<Option<Vec<u8>>> {
    let source_key = format!(
        "shell/{}/{}",
        item.category.name,
        item.file.source_rel.display()
    );
    let source = if config.is_external_presets {
        match tokio::fs::read(&item.source_path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "reading shell preset source: {}",
                        item.source_path.display()
                    )
                });
            }
        }
    } else {
        match crate::presets::read_asset_bytes(&source_key) {
            Some(bytes) => bytes,
            None => return Ok(None),
        }
    };

    if !crate::presets::parse_template_annotation(&source) {
        return Ok(Some(source));
    }

    let env = EnvConfig::load_or_init(config).await?;
    let rendered =
        crate::install_core::apply_transforms(&["template".to_string()], &source, env.as_map())
            .with_context(|| format!("rendering shell template: {}", item.source_path.display()))?;
    Ok(Some(rendered))
}

fn render_diff_or_note(
    current: &[u8],
    expected: &[u8],
    current_label: &str,
    expected_label: &str,
) -> String {
    if current == expected {
        return "No content differences.\n".to_string();
    }

    let current_text = String::from_utf8_lossy(current);
    let expected_text = String::from_utf8_lossy(expected);
    let unified = TextDiff::from_lines(current_text.as_ref(), expected_text.as_ref())
        .unified_diff()
        .context_radius(3)
        .header(current_label, expected_label)
        .to_string();
    colorize_unified_diff(&unified)
}

fn colorize_unified_diff(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    for line in diff.split_inclusive('\n') {
        let colored =
            if line.starts_with("@@") || line.starts_with("---") || line.starts_with("+++") {
                colors::cyan(line.trim_end_matches('\n'))
            } else if line.starts_with('+') && !line.starts_with("+++") {
                colors::green(line.trim_end_matches('\n'))
            } else if line.starts_with('-') && !line.starts_with("---") {
                colors::red(line.trim_end_matches('\n'))
            } else {
                line.trim_end_matches('\n').to_string()
            };
        out.push_str(&colored);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shells::metadata::ShellCategory;

    fn shell_file(category: &str, command: &str, source: &str) -> ShellShowFile {
        ShellShowFile {
            category: ShellCategory {
                name: category.to_string(),
                description: None,
                files: vec![],
                uses_metadata: true,
            },
            file: crate::shells::metadata::ShellFile {
                source_rel: std::path::PathBuf::from(source),
                command_name: command.to_string(),
                description: vec![],
                needs_source: false,
                runtime: crate::bin_links::LinkRuntime::Native,
                transforms: vec![],
                env: vec![],
            },
            source_path: std::path::PathBuf::from(format!("/tmp/{source}")),
            rendered_path: std::path::PathBuf::from(format!("/tmp/rendered/{source}")),
            link_path: std::path::PathBuf::from(format!("/tmp/bin/{command}")),
            link_target: None,
            status: "up-to-date",
        }
    }

    #[test]
    fn app_status_sym_matches_list_semantics() {
        assert_eq!(status_sym(FileStatus::UpToDate), "✓");
        assert_eq!(status_sym(FileStatus::UpdateAvail), "↑");
        assert_eq!(status_sym(FileStatus::UserModified), "~");
        assert_eq!(status_sym(FileStatus::Partial), "~");
        assert_eq!(status_sym(FileStatus::Missing), "!");
        assert_eq!(status_sym(FileStatus::NotInstalled), "✗");
    }

    #[test]
    fn shell_status_sym_matches_list_semantics() {
        assert_eq!(shell_status_sym("up-to-date"), "✓");
        assert_eq!(shell_status_sym("update available"), "↑");
        assert_eq!(shell_status_sym("preset present, bin symlink missing"), "~");
        assert_eq!(shell_status_sym("bin symlink present, script missing"), "~");
        assert_eq!(shell_status_sym("bin symlink present, preset missing"), "~");
        assert_eq!(shell_status_sym("rendered script missing"), "!");
        assert_eq!(shell_status_sym("not installed"), "✗");
    }

    #[test]
    fn diff_note_is_returned_for_equal_content() {
        assert_eq!(
            render_diff_or_note(b"same\n", b"same\n", "current", "expected"),
            "No content differences.\n"
        );
    }

    #[test]
    fn diff_output_contains_headers_and_changed_lines() {
        let diff = render_diff_or_note(b"old\n", b"new\n", "current", "expected");
        assert!(diff.contains("current"));
        assert!(diff.contains("expected"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }

    #[tokio::test]
    async fn embedded_shell_diff_ignores_stale_extracted_source() {
        let dir = std::env::temp_dir().join(format!("shine-show-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let stale_source = dir.join("set_proxy.sh");
        tokio::fs::write(&stale_source, b"#!/bin/bash\necho stale\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);
        let mut item = shell_file("proxy", "setproxy", "set_proxy.sh");
        item.source_path = stale_source;

        let expected = shell_expected_bytes(&config, &item)
            .await
            .unwrap()
            .expect("embedded proxy source should exist");
        let expected = String::from_utf8(expected).unwrap();

        assert!(!expected.contains("echo stale"));
        assert!(expected.contains("PROXY_NO_PROXY=\"localhost,127.0.0.1,::1\""));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
