use owo_colors::{OwoColorize, Stream};

pub(crate) fn symbol(s: &str) -> String {
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

pub(crate) fn green(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.green())
        .to_string()
}

pub(crate) fn yellow(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.yellow())
        .to_string()
}

pub(crate) fn bold(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.bold())
        .to_string()
}

pub(crate) fn dim(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.dimmed())
        .to_string()
}

pub(crate) fn cyan(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.cyan())
        .to_string()
}

pub(crate) fn status_label(s: &str, sym: &str) -> String {
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

/// Returns a formatted note indicating the active external presets directory.
pub(crate) fn external_presets_note(dir: &std::path::Path) -> String {
    use owo_colors::Style;
    let label = "◈ External Presets"
        .if_supports_color(Stream::Stdout, |t| t.style(Style::new().bold().cyan()))
        .to_string();
    format!("{}  {}", label, dir.display())
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
