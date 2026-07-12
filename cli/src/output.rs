use crate::colors;

const SUMMARY_LABEL_WIDTH: usize = 15;
const DETAIL_LABEL_WIDTH: usize = 15;
const DETAIL_STATUS_WIDTH: usize = 13;

/// Tracks whether any upgrade section has printed content yet, so a run
/// covering several optional sections (shell presets, app configs, managed
/// system configs) can emit exactly one blank-line separator between two
/// sections that both printed something — never a leading separator before
/// the first section, and never a stray blank line when a section had
/// nothing to report.
#[derive(Default)]
pub struct SectionSeparator {
    printed: bool,
}

impl SectionSeparator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call immediately before printing a section's header, only on the path
    /// where the section is actually about to print something.
    pub fn begin(&mut self) {
        if self.printed {
            println!();
        }
        self.printed = true;
    }
}

fn join_summary(parts: &[String]) -> String {
    if parts.is_empty() {
        colors::dim("nothing changed")
    } else {
        parts.join(&colors::dim(" · "))
    }
}

/// Appends `"{count} {label}"` (styled with `color`) to `parts` when `count`
/// is nonzero. Collapses the repeated `if count > 0 { parts.push(color(...))
/// }` idiom used when building summary footers from several counters.
pub fn push_count(parts: &mut Vec<String>, count: usize, color: fn(&str) -> String, label: &str) {
    if count > 0 {
        parts.push(color(&format!("{count} {label}")));
    }
}

pub fn summary_line(label: &str, parts: &[String]) {
    println!(
        "{}{}{}",
        colors::bold(label),
        pad(label, SUMMARY_LABEL_WIDTH),
        join_summary(parts)
    );
}

pub fn footer(label: &str, parts: &[String]) {
    println!();
    summary_line(label, parts);
}

pub fn detail_line(label: &str, status: &str, detail: Option<String>) {
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

pub fn hint_line(label: &str, detail: &str) {
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
    fn push_count_appends_only_for_nonzero_counts() {
        let mut parts = Vec::new();
        push_count(&mut parts, 0, colors::green, "up-to-date");
        push_count(&mut parts, 3, colors::green, "up-to-date");
        assert_eq!(parts, vec!["3 up-to-date".to_string()]);
    }

    #[test]
    fn visible_len_ignores_ansi_sequences() {
        assert_eq!(visible_len_without_ansi("\x1b[32mupdated\x1b[0m"), 7);
    }
}
