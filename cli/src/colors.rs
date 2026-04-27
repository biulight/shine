use owo_colors::{OwoColorize, Stream};

/// Colorize a status symbol for stdout output.
/// Automatically degrades to plain text when stdout is not a TTY or `NO_COLOR` is set.
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

/// Wrap a message string in green (success).
pub(crate) fn green(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.green())
        .to_string()
}

/// Wrap a message string in yellow (warning / info).
pub(crate) fn yellow(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |t| t.yellow())
        .to_string()
}
