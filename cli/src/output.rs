use crate::colors;

const SUMMARY_LABEL_WIDTH: usize = 15;
const DETAIL_LABEL_WIDTH: usize = 15;
const DETAIL_STATUS_WIDTH: usize = 13;

fn join_summary(parts: &[String]) -> String {
    if parts.is_empty() {
        colors::dim("nothing changed")
    } else {
        parts.join(&colors::dim(" · "))
    }
}

pub(crate) fn summary_line(label: &str, parts: &[String]) {
    println!(
        "{}{}{}",
        colors::bold(label),
        pad(label, SUMMARY_LABEL_WIDTH),
        join_summary(parts)
    );
}

pub(crate) fn footer(label: &str, parts: &[String]) {
    println!();
    summary_line(label, parts);
}

pub(crate) fn detail_line(label: &str, status: &str, detail: Option<String>) {
    let detail = detail
        .filter(|value| !value.is_empty())
        .map(|value| format!("  {}", colors::dim(&value)))
        .unwrap_or_default();

    println!(
        "{}{}{}{}{}",
        colors::bold(label),
        pad(label, DETAIL_LABEL_WIDTH),
        status,
        pad_plain(visible_len_without_ansi(status), DETAIL_STATUS_WIDTH),
        detail
    );
}

pub(crate) fn hint_line(label: &str, detail: &str) {
    println!(
        "{}{}{}",
        colors::bold(label),
        pad(label, DETAIL_LABEL_WIDTH),
        colors::dim(detail)
    );
}

fn pad(label: &str, width: usize) -> String {
    pad_plain(label.chars().count(), width)
}

fn pad_plain(visible_len: usize, width: usize) -> String {
    " ".repeat(width.saturating_sub(visible_len) + 1)
}

// Only handles CSI sequences (\x1b[...alpha). OSC and other escape types are not stripped.
fn visible_len_without_ansi(value: &str) -> usize {
    let mut len = 0usize;
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }

    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_summary_uses_default_for_empty_parts() {
        assert_eq!(join_summary(&[]), "nothing changed");
    }

    #[test]
    fn visible_len_ignores_ansi_sequences() {
        assert_eq!(visible_len_without_ansi("\x1b[32mupdated\x1b[0m"), 7);
    }
}
