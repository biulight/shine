use super::collect::{AppInfoFile, ShellInfoFile};
use crate::apps::{AppCategory, AppFile};
use crate::colors;
use crate::config::Config;
use crate::output::{colorize_unified_diff, escape_controls, unified_text_diff};
use crate::path_display;
use crate::status::{FileStatus, UpdateChange};
use anyhow::{Context, Result};
use std::borrow::Cow;
use std::path::Path;

pub(super) async fn print_app_file(
    config: &Config,
    item: &AppInfoFile,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    let label = escape_controls(&fallback_app_label(&item.category, &item.file));
    let source_path = config.preset_path(
        Path::new("app")
            .join(&item.category.name)
            .join(&item.file.source_rel),
    );

    println!("{}", colors::bold(&format!("App Config: {label}")));
    if let Some(desc) = &item.file.description {
        println!("{}  {}", colors::dim("Description"), escape_controls(desc));
    } else if let Some(desc) = &item.category.description {
        println!("{}  {}", colors::dim("Description"), escape_controls(desc));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        escape_controls(&path_display::format_home(&source_path, &config.home_dir))
    );
    println!(
        "{}  {}",
        colors::dim("Destination"),
        escape_controls(&path_display::format_home(
            &item.destination,
            &config.home_dir
        ))
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
            escape_controls(&item.file.transforms.join(", "))
        );
    }
    if let Some(entry) = &item.manifest_entry {
        println!("{} {}", colors::dim("Manifest hash"), entry.content_hash);
        if let Some(backup) = &entry.backup {
            println!(
                "{}       {}",
                colors::dim("Backup"),
                escape_controls(&path_display::format_home(backup, &config.home_dir))
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
    let label = escape_controls(&format!(
        "{}/{}",
        item.category.name, item.file.command_name
    ));
    println!("{}", colors::bold(&format!("Shell Preset: {label}")));
    if let Some(desc) = item.file.description.first() {
        println!("{}  {}", colors::dim("Description"), escape_controls(desc));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        escape_controls(&path_display::format_home(
            &item.source_path,
            &config.home_dir
        ))
    );
    if item.installed_source_path != item.source_path {
        println!(
            "{}     {}",
            colors::dim("Snapshot"),
            escape_controls(&path_display::format_home(
                &item.installed_source_path,
                &config.home_dir
            ))
        );
    }
    println!(
        "{}     {}",
        colors::dim("Bin link"),
        escape_controls(&path_display::format_home(
            &item.link_path,
            &config.home_dir
        ))
    );
    if let Some(target) = &item.link_target {
        println!(
            "{} {}",
            colors::dim("Link target"),
            escape_controls(&path_display::format_home(target, &config.home_dir))
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
            escape_controls(&path_display::format_home(
                &item.rendered_path,
                &config.home_dir
            ))
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

pub(crate) async fn print_app_expected_diff(config: &Config, item: &AppInfoFile) -> Result<()> {
    let diff_output = app_diff_output(config, item).await?;
    print_block("Diff", &app_current_path(item), &diff_output);
    Ok(())
}

pub(super) async fn print_shell_update_diff(
    config: &Config,
    item: &ShellInfoFile,
    content_note: Option<&str>,
) -> Result<()> {
    print_update_changes(config, &item.changes);
    if UpdateChange::includes_content(&item.changes) {
        if let Some(note) = content_note {
            println!("     {}", colors::dim(note));
        } else {
            let content_path = shell_content_path(item);
            let diff_output = shell_diff_output(config, item, &content_path).await?;
            print_block("Diff", &content_path, &diff_output);
        }
    } else if !item.changes.is_empty() {
        println!("     {}", colors::dim("content: unchanged"));
    }
    Ok(())
}

pub(super) fn print_shell_snapshot_diff(
    category: &str,
    files: &[shine_core::runtime::ShellSnapshotFileInspection],
) {
    if files.is_empty() {
        return;
    }
    println!();
    println!(
        "{}",
        colors::bold(&escape_controls(&format!(
            "Category snapshot: shell/{category}"
        )))
    );
    for file in files {
        let path = format!("shell/{category}/{}", file.relative_path.display());
        let change = match (&file.current_content, &file.desired_content) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            _ => "changed",
        };
        println!(
            "     {}",
            colors::dim(&escape_controls(&format!("{path}: {change}")))
        );
        if file.current_executable != file.desired_executable {
            let flag = |value: Option<bool>| {
                value.map_or("absent", |flag| if flag { "true" } else { "false" })
            };
            println!(
                "     executable: {} -> {}",
                flag(file.current_executable),
                flag(file.desired_executable)
            );
        }
        let current = file.current_content.as_deref().unwrap_or_default();
        let desired = file.desired_content.as_deref().unwrap_or_default();
        // The added/removed row is the complete difference for an empty file.
        if current != desired {
            print!(
                "{}",
                render_diff_or_note(
                    current,
                    desired,
                    &format!("installed: {path}"),
                    &format!("expected: {path}"),
                )
            );
        }
    }
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
                        escape_controls(&path_display::format_home(destination, &config.home_dir))
                    ))
                );
            }
            UpdateChange::DeploymentChanged { field, from, to } => {
                println!(
                    "     {}",
                    colors::dim(&escape_controls(&format!("{field}: {from} -> {to}")))
                );
            }
            UpdateChange::CommandEntryMissing { path } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "command entry: missing ({})",
                        escape_controls(&path_display::format_home(path, &config.home_dir))
                    ))
                );
            }
            UpdateChange::CommandEntryOutdated { path } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "command entry: does not match expected ({})",
                        escape_controls(&path_display::format_home(path, &config.home_dir))
                    ))
                );
            }
            UpdateChange::ManifestEntryMissing { target } => {
                println!(
                    "     {}",
                    colors::dim(&format!(
                        "manifest entry: missing ({})",
                        escape_controls(target)
                    ))
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
            escape_controls(&path_display::format_home(from, &config.home_dir)),
            escape_controls(&path_display::format_home(to, &config.home_dir))
        ))
    );
}

pub(super) fn shell_content_path(item: &ShellInfoFile) -> std::path::PathBuf {
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
            escape_controls(&path_display::format(path))
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
        FileStatus::GeneratorNotEvaluated => ("generator not evaluated", "!"),
        FileStatus::GeneratorEvaluationFailed => ("generator evaluation failed", "!"),
        FileStatus::GeneratorTrustRequired => ("generator trust required", "!"),
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
        "rendered script missing"
        | "launcher ownership conflict"
        | "preset missing; launcher ownership conflict"
        | "preset missing; installed entry preserved" => "!",
        "not installed" => "✗",
        _ => "~",
    }
}

fn colored_shell_status(status: &str) -> String {
    colors::status_label(status, shell_status_sym(status))
}

async fn app_diff_output(_config: &Config, item: &AppInfoFile) -> Result<String> {
    let Some(expected) = &item.desired_content else {
        return Ok("Unable to render expected content from the active preset.\n".to_string());
    };

    let current_path = app_current_path(item);
    let current: &[u8] = match &item.current_content {
        Some(bytes) => bytes,
        None => &[],
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
            escape_controls(&item.installed_source_path.to_string_lossy())
        ));
    }
    let Some(expected) = &item.desired_content else {
        return Ok("Unable to render expected shell content from the active preset.\n".to_string());
    };

    let Some(current) = &item.current_content else {
        return Ok(format!(
            "Current script is missing: {}.\n",
            escape_controls(&current_path.to_string_lossy())
        ));
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
    let current_style = line_ending_style(current_text);
    let expected_style = line_ending_style(expected_text);
    let mut output = String::new();
    let (current_text, expected_text) = if [current_style, expected_style]
        .contains(&"mixed LF/CRLF")
    {
        // Preserve individual terminators when a file mixes styles; a summary alone would hide
        // changes to their positions. The shared renderer keeps CR visible as escaped content.
        output.push_str(&format!(
            "Line endings: current {current_style}, expected {expected_style} (preserved in diff).\n"
        ));
        (Cow::Borrowed(current_text), Cow::Borrowed(expected_text))
    } else if [current_style, expected_style].contains(&"CRLF") {
        output.push_str(&format!(
            "Line endings: current {current_style}, expected {expected_style} (displayed as LF).\n"
        ));
        // Only presentation copies change. Raw comparisons, hashes and installation bytes stay exact.
        (
            Cow::Owned(current_text.replace("\r\n", "\n")),
            Cow::Owned(expected_text.replace("\r\n", "\n")),
        )
    } else {
        (Cow::Borrowed(current_text), Cow::Borrowed(expected_text))
    };
    if current_text == expected_text {
        output.push_str("Only line endings differ.\n");
    } else {
        output.push_str(&colorize_unified_diff(&unified_text_diff(
            &current_text,
            &expected_text,
            Some((current_label, expected_label)),
        )));
    }
    output
}

fn line_ending_style(text: &str) -> &'static str {
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    match (lf, crlf) {
        (0, _) => "none",
        (_, 0) => "LF",
        _ if lf == crlf => "CRLF",
        _ => "mixed LF/CRLF",
    }
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
    fn diff_escapes_controls_in_headers_and_all_line_kinds() {
        let diff = render_diff_or_note(
            "context\u{7}\nold\u{1b}[2J\t\n尾行\n".as_bytes(),
            "context\u{7}\nnew\u{1b}[H\t\n尾行\n".as_bytes(),
            "current\n--- forged\u{1b}[2J",
            "expected\n@@ forged\tfile",
        );
        for line in [
            "--- current\\n--- forged\\u{1b}[2J",
            "+++ expected\\n@@ forged\\tfile",
            "@@ -1,3 +1,3 @@",
            " context\\u{7}",
            "-old\\u{1b}[2J\\t",
            "+new\\u{1b}[H\\t",
            " 尾行",
        ] {
            assert!(diff.contains(line), "missing {line:?} in {diff:?}");
        }
        assert!(!diff.contains('\u{7}'));
        assert!(!diff.contains('\t'));
        assert!(!diff.contains("\u{1b}[2J"));
        assert!(!diff.contains("\u{1b}[H"));
        assert_eq!(diff.matches('\n').count(), 7);
    }

    #[test]
    fn diff_displays_uniform_crlf_without_churn_and_reports_eol_only_changes() {
        for (current, expected, styles) in [
            (
                "head\r\nold\r\nend\r\n",
                "head\nnew\nend\n",
                "current CRLF, expected LF",
            ),
            (
                "head\nold\nend\n",
                "head\r\nnew\r\nend\r\n",
                "current LF, expected CRLF",
            ),
            (
                "head\r\nold\r\nend\r\n",
                "head\r\nnew\r\nend\r\n",
                "current CRLF, expected CRLF",
            ),
        ] {
            let diff = render_diff_or_note(current.as_bytes(), expected.as_bytes(), "a", "b");
            assert_eq!(
                console::strip_ansi_codes(&diff),
                format!(
                    "Line endings: {styles} (displayed as LF).\n--- a\n+++ b\n@@ -1,3 +1,3 @@\n head\n-old\n+new\n end\n"
                )
            );
        }
        for (current, expected, styles) in [
            ("same\r\n", "same\n", "current CRLF, expected LF"),
            ("same\n", "same\r\n", "current LF, expected CRLF"),
        ] {
            assert_eq!(
                render_diff_or_note(current.as_bytes(), expected.as_bytes(), "a", "b"),
                format!("Line endings: {styles} (displayed as LF).\nOnly line endings differ.\n")
            );
        }
    }

    #[test]
    fn diff_retains_mixed_terminator_positions_instead_of_hiding_them() {
        let diff = render_diff_or_note(b"head\r\nend\n", b"head\nend\r\n", "a", "b");
        assert_eq!(
            console::strip_ansi_codes(&diff),
            "Line endings: current mixed LF/CRLF, expected mixed LF/CRLF (preserved in diff).\n--- a\n+++ b\n@@ -1,2 +1,2 @@\n-head\\r\n-end\n+head\n+end\\r\n"
        );
        assert!(!diff.contains("Only line endings differ"));
    }

    #[test]
    fn diff_keeps_bare_cr_within_lf_lines_and_reports_missing_final_lf() {
        let diff = render_diff_or_note(b"head\r\nold\rtail\r", b"head\r\nnew\rtail\r", "a", "b");
        assert_eq!(
            console::strip_ansi_codes(&diff),
            "Line endings: current CRLF, expected CRLF (displayed as LF).\n--- a\n+++ b\n@@ -1,2 +1,2 @@\n head\n-old\\rtail\\r\n\\ No newline at end of file\n+new\\rtail\\r\n\\ No newline at end of file\n"
        );
        assert!(!diff.contains('\r'));
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
        let runtime = crate::core_runtime::from_config(&config).await.unwrap();
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
