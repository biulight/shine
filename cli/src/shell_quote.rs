//! CLI display policy over the shared literal encoders.

/// Single-quotes `value` for POSIX shells. The only character a single-quoted string cannot
/// contain is `'`, which is closed, escaped, and reopened as `'\''`.
pub(crate) use shine_core::shell_quote::posix_single_quote as single_quote;

/// Renders `value` back into a copy-paste-safe shell argument: left bare when every character
/// is inert to POSIX shells, single-quoted otherwise.
pub(crate) fn quote_if_needed(value: &str) -> String {
    if !value.is_empty() && value.chars().all(is_shell_safe) {
        value.to_string()
    } else {
        single_quote(value)
    }
}

/// One argument in a suggested command: PowerShell on Windows, POSIX elsewhere.
/// This is display policy; execution continues to use the original argv.
pub(crate) fn command_arg(value: &str) -> String {
    if !cfg!(windows) {
        return quote_if_needed(value);
    }
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
    {
        value.to_owned()
    } else {
        shine_core::shell_quote::powershell_single_quote(value)
    }
}

/// Final positional operands in a suggested Shine command. Put options before
/// this text so `--` can protect leading hyphens without changing their values.
pub(crate) fn command_positionals(values: &[&str]) -> String {
    let operands = values
        .iter()
        .map(|value| command_arg(value))
        .collect::<Vec<_>>()
        .join(" ");
    if values.iter().any(|value| value.starts_with('-')) {
        format!("-- {operands}")
    } else {
        operands
    }
}

/// Characters that never need quoting: alphanumerics plus punctuation that is inert to POSIX
/// shells and common in paths, URLs, and rsync targets (e.g. `host:/var/www/`).
fn is_shell_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '-' | '.' | '/' | ':' | ',' | '=' | '@' | '%' | '+')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_if_needed_leaves_plain_values_unquoted() {
        assert_eq!(quote_if_needed("hello"), "hello");
        assert_eq!(quote_if_needed("host:/var/www/"), "host:/var/www/");
    }

    #[test]
    fn quote_if_needed_quotes_empty_and_unsafe_values() {
        assert_eq!(quote_if_needed(""), "''");
        assert_eq!(quote_if_needed("it's"), "'it'\\''s'");
        assert_eq!(quote_if_needed("a b"), "'a b'");
    }
}
