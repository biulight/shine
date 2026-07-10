use owo_colors::{OwoColorize, Stream};

use crate::path_display;

pub fn symbol(s: &str) -> String {
    match s {
        "✓" => s
            .if_supports_color(Stream::Stdout, |t| t.green())
            .to_string(),
        "↑" => s
            .if_supports_color(Stream::Stdout, |t| t.cyan())
            .to_string(),
        "~" => s
            .if_supports_color(Stream::Stdout, |t| t.yellow())
            .to_string(),
        "!" => s
            .if_supports_color(Stream::Stdout, |t| t.magenta())
            .to_string(),
        "✗" => s.if_supports_color(Stream::Stdout, |t| t.red()).to_string(),
        other => other.to_string(),
    }
}

pub fn green(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.green())
        .to_string()
}

pub fn yellow(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.yellow())
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

/// Shared column width for the presets-note labels below, so their paths line up
/// even though "◈ External Presets" and "◈ Presets Overlay" differ in length.
const PRESETS_NOTE_LABEL_WIDTH: usize = 18; // "◈ External Presets".chars().count()

fn presets_note(plain_label: &str, styled_label: &str, dir: &std::path::Path) -> String {
    let pad = " ".repeat(PRESETS_NOTE_LABEL_WIDTH.saturating_sub(plain_label.chars().count()) + 2);
    format!("{styled_label}{pad}{}", path_display::format(dir))
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
}
