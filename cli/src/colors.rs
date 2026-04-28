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
