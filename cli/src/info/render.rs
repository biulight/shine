use super::collect::{AppInfoFile, ShellInfoFile};
use crate::apps::{AppCategory, AppFile};
use crate::colors;
use crate::config::Config;
use crate::path_display;
use crate::status::{FileStatus, UpdateChange};
use anyhow::{Context, Result};
use similar::TextDiff;
use std::path::Path;

pub(super) async fn print_app_file(
    config: &Config,
    item: &AppInfoFile,
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
        print_block("Diff", &app_current_path(item), &diff_output);
    }
    if verbose {
        print_file_content(
            &item.destination,
            "Content",
            item.current_content.as_deref(),
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn print_shell_file(
    config: &Config,
    item: &ShellInfoFile,
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
    if item.installed_source_path != item.source_path {
        println!(
            "{}     {}",
            colors::dim("Snapshot"),
            path_display::format_home(&item.installed_source_path, &config.home_dir)
        );
    }
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
        print_file_content(&content_path, "Content", item.current_content.as_deref()).await?;
    }
    Ok(())
}

pub(super) async fn print_app_update_diff(config: &Config, item: &AppInfoFile) -> Result<()> {
    print_update_changes(config, &item.changes);
    if UpdateChange::includes_content(&item.changes) {
        let diff_output = app_diff_output(config, item).await?;
        print_block("Diff", &app_current_path(item), &diff_output);
    } else if !item.changes.is_empty() {
        println!("     {}", colors::dim("content: unchanged"));
    }
    Ok(())
}

pub(super) async fn print_shell_update_diff(config: &Config, item: &ShellInfoFile) -> Result<()> {
    print_update_changes(config, &item.changes);
    if UpdateChange::includes_content(&item.changes) {
        let content_path = shell_content_path(item);
        let diff_output = shell_diff_output(config, item, &content_path).await?;
        print_block("Diff", &content_path, &diff_output);
    } else if !item.changes.is_empty() {
        println!("     {}", colors::dim("content: unchanged"));
    }
    Ok(())
}

fn print_update_changes(config: &Config, changes: &[UpdateChange]) {
    for change in changes {
        match change {
            UpdateChange::ContentChanged => {
                println!("     {}", colors::dim("content: changed"));
            }
            UpdateChange::SourceRelocated { from, to } => {
                print_path_change(config, "source", from, to);
            }
            UpdateChange::DestinationRelocated { from, to } => {
                print_path_change(config, "destination", from, to);
            }
            UpdateChange::NewFile { destination } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "new file: {}",
                        path_display::format_home(destination, &config.home_dir)
                    ))
                );
            }
            UpdateChange::DeploymentChanged { field, from, to } => {
                println!("     {}", colors::dim(&format!("{field}: {from} -> {to}")));
            }
            UpdateChange::CommandEntryMissing { path } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "command entry: missing ({})",
                        path_display::format_home(path, &config.home_dir)
                    ))
                );
            }
            UpdateChange::CommandEntryOutdated { path } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "command entry: does not match expected ({})",
                        path_display::format_home(path, &config.home_dir)
                    ))
                );
            }
            UpdateChange::ManifestEntryMissing { target } => {
                println!(
                    "     {}",
                    colors::dim(&format!("manifest entry: missing ({target})"))
                );
            }
        }
    }
}

fn print_path_change(config: &Config, field: &str, from: &Path, to: &Path) {
    println!(
        "     {}",
        colors::dim(&format!(
            "{field}: {} -> {}",
            path_display::format_home(from, &config.home_dir),
            path_display::format_home(to, &config.home_dir)
        ))
    );
}

fn shell_content_path(item: &ShellInfoFile) -> std::path::PathBuf {
    item.link_target
        .as_ref()
        .filter(|target| target.starts_with(&item.rendered_path) || **target == item.rendered_path)
        .cloned()
        .or_else(|| {
            item.rendered_path
                .exists()
                .then(|| item.rendered_path.clone())
        })
        .or_else(|| {
            item.changes.iter().find_map(|change| match change {
                UpdateChange::SourceRelocated { from, .. } => Some(from.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| item.installed_source_path.clone())
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

async fn print_file_content(path: &Path, heading: &str, bytes: Option<&[u8]>) -> Result<()> {
    print_heading(heading, path);
    let bytes = bytes.with_context(|| format!("reading installed content: {}", path.display()))?;
    print!("{}", String::from_utf8_lossy(bytes));
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
        "up-to-date" | "live source" | "rendered on next run" => "✓",
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

async fn app_diff_output(_config: &Config, item: &AppInfoFile) -> Result<String> {
    if item
        .file
        .generator
        .as_ref()
        .is_some_and(|generator| !generator.auto)
    {
        return Ok(
            "Expected content is an explicitly refreshed generator snapshot; \
             run `shine app refresh` to materialize it without polling during info.\n"
                .to_string(),
        );
    }
    let expected = match &item.desired_content {
        Some(bytes) => bytes,
        None => {
            return Ok("Unable to render expected content from the active preset.\n".to_string());
        }
    };

    let current_path = app_current_path(item);
    let current = match &item.current_content {
        Some(bytes) => bytes,
        None => {
            return Ok(format!(
                "Current file is missing: {}.\n",
                current_path.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        current,
        expected,
        &current_path.to_string_lossy(),
        &format!(
            "expected: app/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

fn app_current_path(item: &AppInfoFile) -> std::path::PathBuf {
    item.manifest_entry
        .as_ref()
        .map(|entry| entry.destination.clone())
        .unwrap_or_else(|| item.destination.clone())
}

async fn shell_diff_output(
    config: &Config,
    item: &ShellInfoFile,
    current_path: &Path,
) -> Result<String> {
    if config.is_external_presets
        && config.external_shell_mode == crate::config::ExternalShellMode::Snapshot
        && item.current_content.is_none()
    {
        return Ok(format!(
            "Managed snapshot is missing: {}. Run `shine upgrade` to migrate the installed command.\n",
            item.installed_source_path.display()
        ));
    }
    let expected = match &item.desired_content {
        Some(bytes) => bytes,
        None => {
            return Ok(
                "Unable to render expected shell content from the active preset.\n".to_string(),
            );
        }
    };

    let current = match &item.current_content {
        Some(bytes) => bytes,
        None => {
            return Ok(format!(
                "Current script is missing: {}.\n",
                current_path.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        current,
        expected,
        &current_path.to_string_lossy(),
        &format!(
            "expected: shell/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

fn render_diff_or_note(
    current: &[u8],
    expected: &[u8],
    current_label: &str,
    expected_label: &str,
) -> String {
    const MAX_INLINE_DIFF_BYTES: usize = 256 * 1024;

    if current == expected {
        return "No content differences.\n".to_string();
    }

    if current.len() > MAX_INLINE_DIFF_BYTES || expected.len() > MAX_INLINE_DIFF_BYTES {
        return format!(
            "Content diff omitted: inline diff limit is 256 KiB (current: {} bytes, expected: {} bytes).\n",
            current.len(),
            expected.len()
        );
    }
    if current.contains(&0) || expected.contains(&0) {
        return format!(
            "Content diff omitted: binary content contains NUL bytes (current: {} bytes, expected: {} bytes).\n",
            current.len(),
            expected.len()
        );
    }

    let (Ok(current_text), Ok(expected_text)) =
        (std::str::from_utf8(current), std::str::from_utf8(expected))
    else {
        return format!(
            "Content diff omitted: content is not valid UTF-8 (current: {} bytes, expected: {} bytes).\n",
            current.len(),
            expected.len()
        );
    };
    let unified = TextDiff::from_lines(current_text, expected_text)
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

    #[test]
    fn diff_at_inline_limit_is_rendered() {
        let mut current = vec![b'a'; 256 * 1024];
        let mut expected = current.clone();
        current[0] = b'b';
        expected[0] = b'c';

        let diff = render_diff_or_note(&current, &expected, "current", "expected");
        assert!(diff.contains("--- current"));
        assert!(!diff.contains("omitted"));
    }

    #[test]
    fn oversized_diff_is_omitted() {
        let current = vec![b'a'; 256 * 1024 + 1];
        let expected = vec![b'b'; 256 * 1024 + 1];

        let diff = render_diff_or_note(&current, &expected, "current", "expected");
        assert!(diff.contains("inline diff limit is 256 KiB"));
        assert!(diff.contains("262145 bytes"));
    }

    #[test]
    fn nul_byte_diff_is_omitted() {
        let diff = render_diff_or_note(b"old\0", b"new\0", "current", "expected");
        assert!(diff.contains("binary content contains NUL bytes"));
    }

    #[test]
    fn invalid_utf8_diff_is_omitted() {
        let diff = render_diff_or_note(&[0xff], &[0xfe], "current", "expected");
        assert!(diff.contains("content is not valid UTF-8"));
    }

    #[tokio::test]
    async fn embedded_shell_diff_ignores_stale_extracted_source() {
        let dir = std::env::temp_dir().join(format!("shine-info-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let config = Config::new_for_test(&dir);
        let mut runtime = crate::core_runtime::from_config(&config).await.unwrap();
        runtime.context_mut_for_cli().env = crate::env::EnvConfig::load_or_init(&config)
            .await
            .unwrap()
            .as_map()
            .clone();
        let selected_source = runtime
            .inspect_shells()
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.category.name == "proxy" && item.file.command_name == "setproxy")
            .map(|item| item.source_path)
            .expect("embedded proxy source should exist");
        tokio::fs::create_dir_all(selected_source.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&selected_source, b"stale extracted source\n")
            .await
            .unwrap();

        let item = runtime
            .inspect_shells()
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.category.name == "proxy" && item.file.command_name == "setproxy")
            .expect("embedded proxy source should exist");
        assert_eq!(item.source_path, selected_source);
        let expected = String::from_utf8(item.desired_content.unwrap()).unwrap();

        assert!(!expected.contains("stale extracted source"));
        assert!(expected.contains("localhost,127.0.0.1,::1"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
