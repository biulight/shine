use crate::colors;
use console::measure_text_width;
use std::io::IsTerminal;

const SUMMARY_LABEL_WIDTH: usize = 15;
const DETAIL_LABEL_WIDTH: usize = 15;
const DETAIL_STATUS_WIDTH: usize = 13;
const COLUMN_GAP: usize = 2;

/// Tracks whether any upgrade section has printed content yet, so a run
/// covering several optional sections (shell presets, app configs, managed
/// system configs) can emit exactly one blank-line separator between two
/// sections that both printed something — never a leading separator before
/// the first section, and never a stray blank line when a section had
/// nothing to report.
#[derive(Default)]
pub struct SectionSeparator {
    printed: bool,
    preamble: Option<String>,
}

impl SectionSeparator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a separator that prints `preamble` immediately before its first
    /// visible section. If no section begins, the preamble stays hidden.
    pub fn with_preamble(preamble: impl Into<String>) -> Self {
        Self {
            printed: false,
            preamble: Some(preamble.into()),
        }
    }

    /// Call immediately before printing a section's header, only on the path
    /// where the section is actually about to print something.
    pub fn begin(&mut self) {
        if self.printed {
            println!();
        } else if let Some(preamble) = self.preamble.take() {
            println!("{preamble}");
        }
        self.printed = true;
    }

    pub fn has_printed(&self) -> bool {
        self.printed
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

/// Prints names in Homebrew-style columns when stdout is an interactive
/// terminal, falling back to one item per line for redirected output.
pub fn print_columns(items: &[String]) {
    let terminal_width = std::io::stdout().is_terminal().then(|| {
        let width = usize::from(console::Term::stdout().size().1);
        width.max(1)
    });
    print!("{}", format_columns(items, terminal_width));
}

/// Formats names in top-to-bottom, then left-to-right columns.
///
/// `terminal_width = None` represents non-interactive output and intentionally
/// produces one item per line, matching Homebrew's pipe-friendly fallback.
fn format_columns(items: &[String], terminal_width: Option<usize>) -> String {
    if items.is_empty() {
        return String::new();
    }

    let Some(terminal_width) = terminal_width else {
        return items.join("\n") + "\n";
    };
    let max_width = items
        .iter()
        .map(|item| measure_text_width(item))
        .max()
        .unwrap_or(0);
    let mut columns = (terminal_width + COLUMN_GAP) / (max_width + COLUMN_GAP);
    if columns < 2 {
        return items.join("\n") + "\n";
    }

    columns = columns.min(items.len());
    let rows = items.len().div_ceil(columns);
    columns = items.len().div_ceil(rows);
    let column_width = ((terminal_width + COLUMN_GAP) / columns) - COLUMN_GAP;
    let mut output = String::new();

    for row in 0..rows {
        let indices: Vec<usize> = (row..items.len()).step_by(rows).collect();
        for (position, index) in indices.iter().enumerate() {
            let item = &items[*index];
            output.push_str(item);
            if position + 1 < indices.len() {
                let padding = column_width.saturating_sub(measure_text_width(item)) + COLUMN_GAP;
                output.push_str(&" ".repeat(padding));
            }
        }
        output.push('\n');
    }

    output
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

    #[test]
    fn columns_fall_back_to_one_item_per_line_without_a_terminal() {
        let items = vec!["alpha".to_string(), "beta".to_string()];

        assert_eq!(format_columns(&items, None), "alpha\nbeta\n");
    }

    #[test]
    fn columns_fill_top_to_bottom_then_left_to_right() {
        let items = ["alpha", "beta", "gamma", "delta"]
            .map(str::to_string)
            .to_vec();

        assert_eq!(
            format_columns(&items, Some(20)),
            "alpha      gamma\nbeta       delta\n"
        );
    }

    #[test]
    fn columns_fall_back_when_the_terminal_is_too_narrow() {
        let items = vec!["alpha".to_string(), "beta".to_string()];

        assert_eq!(format_columns(&items, Some(7)), "alpha\nbeta\n");
    }

    #[test]
    fn columns_measure_unicode_display_width_and_omit_trailing_spaces() {
        let items = ["猫", "dog", "鸟", "fox"].map(str::to_string).to_vec();
        let rendered = format_columns(&items, Some(12));

        assert_eq!(rendered, "猫     鸟\ndog    fox\n");
        assert!(rendered.lines().all(|line| !line.ends_with(' ')));
    }

    #[test]
    fn preamble_stays_pending_until_the_first_section() {
        let mut separator = SectionSeparator::with_preamble("Upgrade");
        assert!(!separator.has_printed());
        separator.begin();
        assert!(separator.has_printed());
    }
}
