//! Shared POSIX shell-quoting helpers for rendering commands back into copy-paste-safe or
//! sourceable shell syntax. Display-only: `task::mod` and `apps::upgrade` never execute the
//! quoted string itself, they always run the underlying argv directly via
//! `std::process::Command`/`tokio::process::Command`, so this module has no injection surface
//! of its own.

/// Single-quotes `value` for POSIX shells. The only character a single-quoted string cannot
/// contain is `'`, which is closed, escaped, and reopened as `'\''`.
pub(crate) fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Renders `value` back into a copy-paste-safe shell argument: left bare when every character
/// is inert to POSIX shells, single-quoted otherwise.
pub(crate) fn quote_if_needed(value: &str) -> String {
    if !value.is_empty() && value.chars().all(is_shell_safe) {
        value.to_string()
    } else {
        single_quote(value)
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

    #[test]
    fn single_quote_escapes_embedded_quotes() {
        assert_eq!(single_quote("it's"), "'it'\\''s'");
        assert_eq!(single_quote(""), "''");
    }
}
