//! Shared terminal presentation for Preset diagnostics.

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
    let continuation_prefix = format!("{prefix}{}", " ".repeat(visible_width(&symbol)));
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
        .saturating_sub(visible_width(first_prefix))
        .max(20);
    let continuation_available = TEXT_WIDTH
        .saturating_sub(visible_width(continuation_prefix))
        .max(20);
    let mut line = String::new();
    let mut prefix = first_prefix;
    let mut available = first_available;
    for word in text.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        if !line.is_empty() && line.chars().count() + separator + word.chars().count() > available {
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

fn visible_width(value: &str) -> usize {
    let mut escaped = false;
    value
        .chars()
        .filter(|character| {
            if *character == '\u{1b}' {
                escaped = true;
                return false;
            }
            if escaped {
                if character.is_ascii_alphabetic() {
                    escaped = false;
                }
                return false;
            }
            true
        })
        .count()
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
        assert!(output.lines().all(|line| visible_width(line) <= TEXT_WIDTH));
        assert!(output.contains("\n    word"));
    }
}
