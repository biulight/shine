//! Shared terminal presentation for Preset diagnostics.

use console::measure_text_width;
use shine_core::runtime::{PresetDiagnostic, PresetDiagnosticSeverity};
use std::fmt::Write as _;

const TEXT_WIDTH: usize = 96;

pub(crate) fn count_phrase(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

pub(crate) fn diagnostic_symbol(severity: PresetDiagnosticSeverity) -> String {
    match severity {
        PresetDiagnosticSeverity::Error => crate::colors::symbol("✗"),
        PresetDiagnosticSeverity::Warning => crate::colors::yellow("!"),
    }
}

pub(crate) fn write_diagnostic(
    output: &mut String,
    prefix: &str,
    diagnostic: &PresetDiagnostic,
    include_symbol: bool,
    include_path: bool,
) {
    let symbol = if include_symbol {
        format!("{} ", diagnostic_symbol(diagnostic.severity))
    } else {
        String::new()
    };
    let message_prefix = format!("{prefix}{symbol}");
    let continuation_prefix = format!("{prefix}{}", " ".repeat(measure_text_width(&symbol)));
    write_wrapped_with_continuation(
        output,
        &message_prefix,
        &continuation_prefix,
        &diagnostic.message,
    );
    let detail_prefix = format!("{prefix}  ");
    let _ = writeln!(
        output,
        "{detail_prefix}{} {}",
        crate::colors::dim("code:"),
        diagnostic.code
    );
    if include_path && let Some(path) = &diagnostic.path {
        let _ = writeln!(
            output,
            "{detail_prefix}{} {}",
            crate::colors::dim("file:"),
            path.display()
        );
    }
}

pub(crate) fn write_wrapped(output: &mut String, prefix: &str, text: &str) {
    write_wrapped_with_continuation(output, prefix, prefix, text);
}

fn write_wrapped_with_continuation(
    output: &mut String,
    first_prefix: &str,
    continuation_prefix: &str,
    text: &str,
) {
    let first_available = TEXT_WIDTH
        .saturating_sub(measure_text_width(first_prefix))
        .max(20);
    let continuation_available = TEXT_WIDTH
        .saturating_sub(measure_text_width(continuation_prefix))
        .max(20);
    let mut line = String::new();
    let mut prefix = first_prefix;
    let mut available = first_available;
    for word in text.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        if !line.is_empty()
            && measure_text_width(&line) + separator + measure_text_width(word) > available
        {
            let _ = writeln!(output, "{prefix}{line}");
            line.clear();
            prefix = continuation_prefix;
            available = continuation_available;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if line.is_empty() {
        let _ = writeln!(output, "{prefix}");
    } else {
        let _ = writeln!(output, "{prefix}{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_diagnostic_indents_continuations_without_repeating_the_symbol() {
        let diagnostic = PresetDiagnostic {
            severity: PresetDiagnosticSeverity::Error,
            code: "example".to_string(),
            message: "word ".repeat(30),
            path: None,
        };
        let mut output = String::new();

        write_diagnostic(&mut output, "  ", &diagnostic, true, false);

        assert_eq!(output.matches('✗').count(), 1);
        assert!(
            output
                .lines()
                .all(|line| measure_text_width(line) <= TEXT_WIDTH)
        );
        assert!(output.contains("\n    word"));
    }

    #[test]
    fn wrapping_measures_unicode_and_color_without_splitting_long_tokens() {
        for word in ["中文", "e\u{301}", "\x1b[32m中文\x1b[0m"] {
            let text = format!("{word} ").repeat(70);
            let mut output = String::new();
            write_wrapped(&mut output, "  ", &text);
            assert!(output.lines().count() > 1);
            assert!(
                output
                    .lines()
                    .all(|line| measure_text_width(line) <= TEXT_WIDTH),
                "{output}"
            );
            assert_eq!(
                output.split_whitespace().collect::<Vec<_>>(),
                text.split_whitespace().collect::<Vec<_>>()
            );
        }
        let long_path = format!("/{}", "中文".repeat(60));
        let mut output = String::new();
        write_wrapped(&mut output, "  ", &long_path);
        assert_eq!(output, format!("  {long_path}\n"));
    }
}
