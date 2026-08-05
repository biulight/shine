use owo_colors::{OwoColorize, Stream};

use crate::path_display;

fn symbol_for_stream(s: &str, stream: Stream) -> String {
    match s {
        "✓" => s.if_supports_color(stream, |t| t.green()).to_string(),
        "↑" => s.if_supports_color(stream, |t| t.cyan()).to_string(),
        "~" => s.if_supports_color(stream, |t| t.yellow()).to_string(),
        "!" => s.if_supports_color(stream, |t| t.magenta()).to_string(),
        "✗" => s.if_supports_color(stream, |t| t.red()).to_string(),
        other => other.to_string(),
    }
}

pub fn symbol(s: &str) -> String {
    symbol_for_stream(s, Stream::Stdout)
}

/// Like [`symbol`], but checks color support against stderr instead of
/// stdout. Use this when the result is printed with `eprintln!` — checking
/// the wrong stream means color escapes can be wrongly included or omitted
/// when stdout and stderr have different redirection (e.g. `cmd > log.txt`
/// with a real terminal still attached to stderr).
pub fn symbol_stderr(s: &str) -> String {
    symbol_for_stream(s, Stream::Stderr)
}

pub fn green(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.green())
        .to_string()
}

pub fn yellow(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.yellow())
        .to_string()
}

/// Like [`yellow`], but checks color support against stderr. See
/// [`symbol_stderr`] for why this matters.
pub fn yellow_stderr(s: &str) -> String {
    s.if_supports_color(Stream::Stderr, |t| t.yellow())
        .to_string()
}

pub fn red(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.red()).to_string()
}

pub fn bold(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.bold())
        .to_string()
}

pub fn dim(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.dimmed())
        .to_string()
}

pub fn cyan(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.cyan())
        .to_string()
}

pub fn status_label(s: &str, sym: &str) -> String {
    match sym {
        "✓" => s
            .if_supports_color(Stream::Stdout, |t| t.green())
            .to_string(),
        "↑" => s
            .if_supports_color(Stream::Stdout, |t| t.cyan())
            .to_string(),
        "~" | "!" => s
            .if_supports_color(Stream::Stdout, |t| t.yellow())
            .to_string(),
        "✗" => s.if_supports_color(Stream::Stdout, |t| t.red()).to_string(),
        _ => s.to_string(),
    }
}

/// Shared column width for the presets-note labels below, so their values line up.
const PRESETS_NOTE_LABEL_WIDTH: usize = 18; // "◈ External Presets".chars().count()

fn presets_note_value(plain_label: &str, styled_label: &str, value: &str) -> String {
    let pad = " ".repeat(PRESETS_NOTE_LABEL_WIDTH.saturating_sub(plain_label.chars().count()) + 2);
    format!("{styled_label}{pad}{value}")
}

fn presets_note(plain_label: &str, styled_label: &str, dir: &std::path::Path) -> String {
    presets_note_value(plain_label, styled_label, &path_display::format(dir))
}

/// Returns a formatted note indicating the active external presets directory.
pub fn external_presets_note(dir: &std::path::Path) -> String {
    use owo_colors::Style;
    let label = "◈ External Presets";
    let styled = label
        .if_supports_color(Stream::Stdout, |t| t.style(Style::new().bold().cyan()))
        .to_string();
    presets_note(label, &styled, dir)
}

/// Returns a formatted note indicating the active presets overlay directory.
pub fn presets_overlay_note(dir: &std::path::Path) -> String {
    use owo_colors::Style;
    let label = "◈ Presets Overlay";
    let styled = label
        .if_supports_color(Stream::Stdout, |t| t.style(Style::new().bold().yellow()))
        .to_string();
    presets_note(label, &styled, dir)
}

/// Returns a formatted note describing how external shell presets are deployed.
pub fn shell_deployment_note(value: &str) -> String {
    let label = "◈ Shell Deployment";
    presets_note_value(label, label, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn external_presets_note_contains_path() {
        let path = Path::new("/home/user/.custom/presets");
        let note = external_presets_note(path);
        assert!(
            note.contains("/home/user/.custom/presets"),
            "note should include the path: {note:?}"
        );
    }

    #[test]
    fn external_presets_note_contains_presets_label() {
        let path = Path::new("/some/dir");
        let note = external_presets_note(path);
        assert!(
            note.contains("External Presets"),
            "note should include the 'External Presets' label: {note:?}"
        );
    }

    #[test]
    fn presets_note_values_use_the_same_column() {
        let external = external_presets_note(Path::new("/external"));
        let overlay = presets_overlay_note(Path::new("/overlay"));
        let deployment = shell_deployment_note("snapshot");

        assert_eq!(external.find("/external"), overlay.find("/overlay"));
        assert_eq!(external.find("/external"), deployment.find("snapshot"));
    }
}
